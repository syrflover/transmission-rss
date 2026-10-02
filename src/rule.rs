use std::path::PathBuf;

use serde::Deserialize;

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
