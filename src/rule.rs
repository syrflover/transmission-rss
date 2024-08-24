use std::path::{Path, PathBuf};

use serde::Deserialize;

const fn default_starts_episode_at() -> isize {
    1
}

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
    pub fn test(&self, target: &str) -> bool {
        if self.regex {
            unimplemented!()
        } else if self.case_insensitive {
            target.to_lowercase().contains(&self.r#match.to_lowercase())
        } else {
            target.contains(&self.r#match)
        }
    }

    pub fn directory(&self, base: impl AsRef<Path>) -> PathBuf {
        base.as_ref().join(&self.directory)
    }
}
