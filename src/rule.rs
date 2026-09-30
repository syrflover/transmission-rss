use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::rss::RuleSpec;

const fn default_starts_episode_at() -> isize {
    1
}

/// A download rule as written in the legacy channels YAML.
/// The judgment lives in [`crate::rss::ChannelEvaluator`].
#[derive(Debug, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub case_insensitive: bool,
    #[serde(rename = "match")]
    pub r#match: String,
    #[serde(rename = "episode", default = "default_starts_episode_at")]
    pub starts_episode_at: isize,
    pub(crate) directory: PathBuf,
}

impl Rule {
    pub fn directory(&self, base: impl AsRef<Path>) -> PathBuf {
        base.as_ref().join(&self.directory)
    }
}

impl From<&Rule> for RuleSpec {
    fn from(rule: &Rule) -> Self {
        Self {
            pattern: Some(rule.r#match.clone()),
            regex: rule.regex,
            case_insensitive: rule.case_insensitive,
            directory: rule.directory.clone(),
            episode: rule.starts_episode_at,
        }
    }
}
