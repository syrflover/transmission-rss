use std::path::{Path, PathBuf};

use regex::{Regex, RegexBuilder};

/// A rule as the evaluation sees it. Rules are identified by their index in
/// [`ChannelSpec::rules`], which is also their priority (earlier wins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSpec {
    /// The match phrase. `None` is a title-waiting subscription and matches nothing.
    /// `Some("")` is an ordinary empty substring or empty regex and matches every title.
    pub pattern: Option<String>,
    /// Treat `pattern` as a regular expression instead of a substring.
    pub regex: bool,
    pub case_insensitive: bool,
    /// Joined onto the channel directory to form the save path.
    pub directory: PathBuf,
    /// Carried through to the caller for episode renaming; not used by the evaluation.
    pub episode: isize,
}

/// A channel's excludes, base directory and ordered rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSpec {
    pub directory: PathBuf,
    /// Case-sensitive substrings; a title containing any of them is excluded.
    pub excludes: Vec<String>,
    pub rules: Vec<RuleSpec>,
}

/// A rule whose regular expression does not compile. That rule matches nothing;
/// the channel's other rules keep being evaluated.
#[derive(Debug, Clone, thiserror::Error)]
#[error("rule {rule}: {source}")]
pub struct RuleError {
    /// Index into [`ChannelSpec::rules`].
    pub rule: usize,
    #[source]
    pub source: regex::Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The title contains one of the channel's `excludes`.
    ChannelExcluded,
    /// No rule matched the title.
    NoRuleMatched,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Selected {
        /// Index of the first matching rule.
        rule: usize,
        /// `channel.directory.join(rule.directory)`.
        save_path: PathBuf,
        /// The applied rule's `episode`, carried through unchanged.
        episode: isize,
    },
    Skipped(SkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub outcome: Outcome,
    /// Rules after the applied one that also match the title, in order.
    /// Empty unless the item is selected.
    pub overlapping: Vec<usize>,
}

impl Evaluation {
    pub fn is_selected(&self) -> bool {
        matches!(self.outcome, Outcome::Selected { .. })
    }

    /// Index of the applied rule, if selected.
    pub fn applied_rule(&self) -> Option<usize> {
        match self.outcome {
            Outcome::Selected { rule, .. } => Some(rule),
            Outcome::Skipped(_) => None,
        }
    }
}

enum Matcher {
    /// No match phrase, or a regex that failed to compile.
    Never,
    Substring(String),
    /// Compared after lowercasing both sides; the phrase is stored lowercased.
    SubstringIgnoreCase(String),
    Regex(Regex),
}

impl Matcher {
    fn new(rule: &RuleSpec) -> Result<Self, regex::Error> {
        let Some(pattern) = rule.pattern.as_deref() else {
            return Ok(Self::Never);
        };

        Ok(if rule.regex {
            Self::Regex(
                RegexBuilder::new(pattern)
                    .case_insensitive(rule.case_insensitive)
                    .build()?,
            )
        } else if rule.case_insensitive {
            Self::SubstringIgnoreCase(pattern.to_lowercase())
        } else {
            Self::Substring(pattern.to_owned())
        })
    }

    fn is_match(&self, title: &str) -> bool {
        match self {
            Self::Never => false,
            Self::Substring(x) => title.contains(x.as_str()),
            Self::SubstringIgnoreCase(x) => title.to_lowercase().contains(x.as_str()),
            Self::Regex(re) => re.is_match(title),
        }
    }
}

/// Where a rule saves what it selects: the channel's directory with the
/// rule's directory joined on. The evaluation and a retry of a failed item
/// (which goes where its rule would have put it) both come here.
pub fn save_path(channel_directory: &Path, rule_directory: &Path) -> PathBuf {
    channel_directory.join(rule_directory)
}

/// A [`ChannelSpec`] with its regular expressions compiled once, ready to evaluate many titles.
pub struct ChannelEvaluator {
    spec: ChannelSpec,
    matchers: Vec<Matcher>,
    errors: Vec<RuleError>,
}

impl ChannelEvaluator {
    pub fn new(spec: ChannelSpec) -> Self {
        let mut matchers = Vec::with_capacity(spec.rules.len());
        let mut errors = Vec::new();

        for (rule, r) in spec.rules.iter().enumerate() {
            match Matcher::new(r) {
                Ok(matcher) => matchers.push(matcher),
                Err(source) => {
                    matchers.push(Matcher::Never);
                    errors.push(RuleError { rule, source });
                }
            }
        }

        Self {
            spec,
            matchers,
            errors,
        }
    }

    /// Rules whose regular expression failed to compile, in rule order.
    pub fn rule_errors(&self) -> &[RuleError] {
        &self.errors
    }

    pub fn spec(&self) -> &ChannelSpec {
        &self.spec
    }

    pub fn evaluate(&self, title: &str) -> Evaluation {
        if self
            .spec
            .excludes
            .iter()
            .any(|ex| title.contains(ex.as_str()))
        {
            return Evaluation {
                outcome: Outcome::Skipped(SkipReason::ChannelExcluded),
                overlapping: Vec::new(),
            };
        }

        let mut matching = self
            .matchers
            .iter()
            .enumerate()
            .filter(|(_, matcher)| matcher.is_match(title))
            .map(|(i, _)| i);

        let Some(rule) = matching.next() else {
            return Evaluation {
                outcome: Outcome::Skipped(SkipReason::NoRuleMatched),
                overlapping: Vec::new(),
            };
        };

        let applied = &self.spec.rules[rule];

        Evaluation {
            outcome: Outcome::Selected {
                rule,
                save_path: save_path(&self.spec.directory, &applied.directory),
                episode: applied.episode,
            },
            overlapping: matching.collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn rule(pattern: Option<&str>) -> RuleSpec {
        RuleSpec {
            pattern: pattern.map(str::to_owned),
            regex: false,
            case_insensitive: false,
            directory: PathBuf::from("Show"),
            episode: 1,
        }
    }

    fn channel(excludes: &[&str], rules: Vec<RuleSpec>) -> ChannelEvaluator {
        ChannelEvaluator::new(ChannelSpec {
            directory: PathBuf::from("/media/anime"),
            excludes: excludes.iter().map(|x| x.to_string()).collect(),
            rules,
        })
    }

    #[test]
    fn channel_exclude_wins_over_matching_rule() {
        let ev = channel(&["[Batch]"], vec![rule(Some("Show"))]);

        let res = ev.evaluate("[Group] Show - 01 [Batch]");

        assert_eq!(res.outcome, Outcome::Skipped(SkipReason::ChannelExcluded));
        assert!(!res.is_selected());
        assert!(res.overlapping.is_empty());
        // the same title without the excluded text is selected
        assert!(ev.evaluate("[Group] Show - 01").is_selected());
    }

    #[test]
    fn channel_exclude_is_case_sensitive() {
        let ev = channel(&["[Batch]"], vec![rule(Some("Show"))]);

        assert!(ev.evaluate("[Group] Show - 01 [batch]").is_selected());
    }

    #[test]
    fn case_insensitive_rule_ignores_case() {
        let mut insensitive = rule(Some("sHoW"));
        insensitive.case_insensitive = true;
        let ev = channel(&[], vec![insensitive]);
        assert!(ev.evaluate("[Group] SHOW - 01").is_selected());

        let ev = channel(&[], vec![rule(Some("sHoW"))]);
        assert_eq!(
            ev.evaluate("[Group] SHOW - 01").outcome,
            Outcome::Skipped(SkipReason::NoRuleMatched)
        );
    }

    #[test]
    fn first_rule_applies_and_later_matching_rules_are_reported() {
        let mut second = rule(Some("Show"));
        second.directory = PathBuf::from("Other");
        let ev = channel(
            &[],
            vec![
                rule(Some("nothing")),
                rule(Some("Show - 01")),
                second,
                rule(Some("[Group]")),
            ],
        );

        let res = ev.evaluate("[Group] Show - 01");

        assert_eq!(res.applied_rule(), Some(1));
        assert_eq!(res.overlapping, vec![2, 3]);
        assert_eq!(
            res.outcome,
            Outcome::Selected {
                rule: 1,
                save_path: PathBuf::from("/media/anime/Show"),
                episode: 1,
            }
        );
    }

    #[test]
    fn single_match_has_no_overlap() {
        let ev = channel(&[], vec![rule(Some("Show")), rule(Some("Other"))]);

        assert!(ev.evaluate("Show - 01").overlapping.is_empty());
    }

    #[test]
    fn regex_rule_matches_by_pattern() {
        let mut r = rule(Some(r"^\[SubsPlease\] Sono Bisque Doll - (1[3-9]|2[0-4])"));
        r.regex = true;
        let ev = channel(&[], vec![r]);
        assert!(ev.rule_errors().is_empty());

        assert!(ev
            .evaluate("[SubsPlease] Sono Bisque Doll - 13 (1080p) [ABCD1234].mkv")
            .is_selected());
        assert!(ev
            .evaluate("[SubsPlease] Sono Bisque Doll - 24 (1080p) [ABCD1234].mkv")
            .is_selected());
        assert!(!ev
            .evaluate("[SubsPlease] Sono Bisque Doll - 12 (1080p) [ABCD1234].mkv")
            .is_selected());
        assert!(!ev
            .evaluate("[SubsPlease] Sono Bisque Doll - 25 (1080p) [ABCD1234].mkv")
            .is_selected());
        // anchored: a leading prefix does not match
        assert!(!ev
            .evaluate("x [SubsPlease] Sono Bisque Doll - 13")
            .is_selected());
    }

    #[test]
    fn regex_is_not_a_substring_search() {
        // As a substring the pattern text would only match itself; as a regex `\d+` needs digits.
        let mut r = rule(Some(r"Show - \d+"));
        r.regex = true;
        let ev = channel(&[], vec![r]);

        assert!(ev.evaluate("Show - 12").is_selected());
        assert!(!ev.evaluate(r"Show - \d+").is_selected());
    }

    #[test]
    fn regex_honors_case_insensitive() {
        let mut r = rule(Some(r"^show - \d+$"));
        r.regex = true;
        r.case_insensitive = true;
        let ev = channel(&[], vec![r.clone()]);
        assert!(ev.evaluate("SHOW - 12").is_selected());

        r.case_insensitive = false;
        let ev = channel(&[], vec![r]);
        assert!(!ev.evaluate("SHOW - 12").is_selected());
    }

    #[test]
    fn invalid_regex_is_a_rule_error_and_other_rules_keep_evaluating() {
        let mut broken = rule(Some(r"Show (unclosed"));
        broken.regex = true;
        let mut ok_regex = rule(Some(r"Show - \d+"));
        ok_regex.regex = true;
        let ev = channel(&[], vec![rule(Some("nope")), broken, ok_regex]);

        let errors = ev.rule_errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].rule, 1);
        assert!(!errors[0].to_string().is_empty());

        // the broken rule matches nothing, not even its own text as a substring
        let res = ev.evaluate("Show (unclosed");
        assert!(!res.is_selected());

        // the later valid rule still applies
        let res = ev.evaluate("Show - 03");
        assert_eq!(res.applied_rule(), Some(2));
        assert!(res.overlapping.is_empty());
    }

    #[test]
    fn invalid_regex_does_not_fall_back_to_substring() {
        let mut broken = rule(Some("Show ("));
        broken.regex = true;
        let ev = channel(&[], vec![broken]);

        assert_eq!(ev.rule_errors().len(), 1);
        assert_eq!(
            ev.evaluate("Show ( - 01").outcome,
            Outcome::Skipped(SkipReason::NoRuleMatched)
        );
    }

    #[test]
    fn rule_without_pattern_matches_nothing() {
        for (regex, case_insensitive) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let mut waiting = rule(None);
            waiting.regex = regex;
            waiting.case_insensitive = case_insensitive;
            let ev = channel(&[], vec![waiting]);

            assert!(ev.rule_errors().is_empty());
            for title in ["", "Show - 01", "[Group] anything"] {
                assert_eq!(
                    ev.evaluate(title).outcome,
                    Outcome::Skipped(SkipReason::NoRuleMatched),
                    "regex={regex} case_insensitive={case_insensitive} title={title:?}"
                );
            }
        }
    }

    #[test]
    fn title_waiting_rule_does_not_shadow_or_overlap() {
        let ev = channel(&[], vec![rule(None), rule(Some("Show")), rule(None)]);

        let res = ev.evaluate("Show - 01");

        assert_eq!(res.applied_rule(), Some(1));
        assert!(res.overlapping.is_empty());
    }

    #[test]
    fn save_path_joins_channel_and_rule_directories() {
        let mut r = rule(Some("Sayonara Lara"));
        r.directory = PathBuf::from("Sayonara Lara/Season 01");
        let ev = channel(&[], vec![r]);

        let Outcome::Selected { save_path, .. } = ev.evaluate("Sayonara Lara - 01").outcome else {
            panic!("expected selection");
        };

        assert_eq!(save_path, Path::new("/media/anime/Sayonara Lara/Season 01"));
    }

    #[test]
    fn episode_is_carried_through_including_negative() {
        let mut r = rule(Some("Show"));
        r.episode = -24;
        let ev = channel(&[], vec![r]);

        let Outcome::Selected { episode, .. } = ev.evaluate("Show - 25").outcome else {
            panic!("expected selection");
        };

        assert_eq!(episode, -24);
    }

    #[test]
    fn empty_pattern_string_matches_every_title_like_the_legacy_substring_test() {
        let ev = channel(&[], vec![rule(Some(""))]);

        assert!(ev.evaluate("anything").is_selected());
    }

    #[test]
    fn channel_without_rules_selects_nothing() {
        let ev = channel(&[], vec![]);

        assert_eq!(
            ev.evaluate("Show").outcome,
            Outcome::Skipped(SkipReason::NoRuleMatched)
        );
    }
}
