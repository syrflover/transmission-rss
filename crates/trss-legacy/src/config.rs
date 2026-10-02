use std::path::PathBuf;

use serde::Deserialize;

use crate::rule::Rule;

/// A channel as written in the legacy channels YAML, which the settings import reads.
#[derive(Debug, Deserialize)]
pub struct ChannelConfig {
    pub url: String,
    pub directory: PathBuf,
    #[serde(default)]
    pub excludes: Vec<String>,
    pub rules: Vec<Rule>,
}
