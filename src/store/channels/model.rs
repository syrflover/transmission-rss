use std::fmt;
use std::path::Path;

use url::Url;

use super::ChannelError;

/// Optimistic-concurrency counter. A new row has version 1 and every change
/// adds 1. Updates carry the version the caller last saw.
pub type Version = i64;

/// Replaces a secret query value in [`mask_url`] output.
pub const MASK: &str = "***";

/// Editable channel fields.
#[derive(Clone, PartialEq, Eq)]
pub struct ChannelInput {
    /// Full URL with the original query values, secret ones included.
    pub url: String,
    /// Base save directory for the channel's rules.
    pub base_dir: String,
    /// Ordered exclude conditions.
    pub excludes: Vec<String>,
    /// Query names whose values are secret. Each must occur in `url`.
    pub secret_query: Vec<String>,
    /// Past-episode search template such as `[SubsPlease] {match} 1080p`.
    pub past_search: Option<String>,
}

impl ChannelInput {
    /// A new channel starts with every query value secret, so nothing leaks
    /// into an export before the user opts a value out.
    pub fn new(url: impl Into<String>, base_dir: impl Into<String>) -> Self {
        let url = url.into();
        ChannelInput {
            secret_query: query_names(&url),
            url,
            base_dir: base_dir.into(),
            excludes: Vec::new(),
            past_search: None,
        }
    }

    pub(super) fn validate(&self) -> Result<(), ChannelError> {
        let parsed = Url::parse(&self.url)
            .map_err(|_| ChannelError::Invalid("channel url is not a valid URL"))?;
        if self.base_dir.is_empty() {
            return Err(ChannelError::Invalid("channel base directory is empty"));
        }
        let names = query_names(parsed.as_str());
        for (i, name) in self.secret_query.iter().enumerate() {
            if !names.contains(name) {
                return Err(ChannelError::Invalid(
                    "secret query name does not occur in the channel url",
                ));
            }
            if self.secret_query[..i].contains(name) {
                return Err(ChannelError::Invalid("secret query name is listed twice"));
            }
        }
        Ok(())
    }
}

impl fmt::Debug for ChannelInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelInput")
            .field("url", &mask_url(&self.url, &self.secret_query))
            .field("base_dir", &self.base_dir)
            .field("excludes", &self.excludes)
            .field("secret_query", &self.secret_query)
            .field("past_search", &self.past_search)
            .finish()
    }
}

/// A stored channel.
#[derive(Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    /// Order among channels, ascending from 0.
    pub position: i64,
    pub version: Version,
    pub url: String,
    pub base_dir: String,
    pub excludes: Vec<String>,
    pub secret_query: Vec<String>,
    pub past_search: Option<String>,
}

impl Channel {
    /// The URL with secret query values masked; the only form meant for
    /// display and logs.
    pub fn masked_url(&self) -> String {
        mask_url(&self.url, &self.secret_query)
    }

    /// The editable fields, e.g. to edit and send back to `update_channel`.
    pub fn to_input(&self) -> ChannelInput {
        ChannelInput {
            url: self.url.clone(),
            base_dir: self.base_dir.clone(),
            excludes: self.excludes.clone(),
            secret_query: self.secret_query.clone(),
            past_search: self.past_search.clone(),
        }
    }
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel")
            .field("id", &self.id)
            .field("position", &self.position)
            .field("version", &self.version)
            .field("url", &self.masked_url())
            .field("base_dir", &self.base_dir)
            .field("excludes", &self.excludes)
            .field("secret_query", &self.secret_query)
            .field("past_search", &self.past_search)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleState {
    Active,
    /// Kept in place but not collected.
    Archived,
}

impl RuleState {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleState::Active => "active",
            RuleState::Archived => "archived",
        }
    }

    pub(super) fn parse(s: &str) -> Option<RuleState> {
        match s {
            "active" => Some(RuleState::Active),
            "archived" => Some(RuleState::Archived),
            _ => None,
        }
    }
}

/// Editable rule fields. The owning channel is not among them: it is fixed
/// when the rule is created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleInput {
    /// Text to look for in a release title. `None` is a rule still waiting for
    /// its title and matches nothing. `Some("")` is rejected.
    pub r#match: Option<String>,
    pub regex: bool,
    pub case_insensitive: bool,
    /// Save directory relative to the channel's base directory.
    pub directory: String,
    /// Episode offset handed to renaming; may be negative.
    pub episode: i64,
    /// Whether `episode` was derived automatically rather than typed by the user.
    pub episode_auto: bool,
    pub state: RuleState,
}

impl Default for RuleInput {
    /// `episode` defaults to 1, as `starts_episode_at` does in the legacy YAML.
    fn default() -> Self {
        RuleInput {
            r#match: None,
            regex: false,
            case_insensitive: false,
            directory: String::new(),
            episode: 1,
            episode_auto: false,
            state: RuleState::Active,
        }
    }
}

impl RuleInput {
    pub(super) fn validate(&self) -> Result<(), ChannelError> {
        if self.r#match.as_deref() == Some("") {
            return Err(ChannelError::Invalid(
                "rule match must be null or non-empty",
            ));
        }
        if Path::new(&self.directory).is_absolute() {
            return Err(ChannelError::Invalid(
                "rule directory must be relative to the channel directory",
            ));
        }
        Ok(())
    }
}

/// A stored rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub channel_id: String,
    /// Order within the channel, ascending from 0; archived rules keep theirs.
    pub position: i64,
    pub version: Version,
    pub r#match: Option<String>,
    pub regex: bool,
    pub case_insensitive: bool,
    pub directory: String,
    pub episode: i64,
    pub episode_auto: bool,
    pub state: RuleState,
}

impl Rule {
    pub fn to_input(&self) -> RuleInput {
        RuleInput {
            r#match: self.r#match.clone(),
            regex: self.regex,
            case_insensitive: self.case_insensitive,
            directory: self.directory.clone(),
            episode: self.episode,
            episode_auto: self.episode_auto,
            state: self.state,
        }
    }
}

/// A channel with all its rules, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelWithRules {
    pub channel: Channel,
    pub rules: Vec<Rule>,
}

/// One entry of a requested order: an item and the version the caller saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderItem {
    pub id: String,
    pub version: Version,
}

/// Distinct query parameter names of `url`, in order of first appearance.
/// Empty when `url` is not a valid URL.
pub fn query_names(url: &str) -> Vec<String> {
    let Ok(parsed) = Url::parse(url) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    for (name, _) in parsed.query_pairs() {
        if !names.iter().any(|n| n == &*name) {
            names.push(name.into_owned());
        }
    }
    names
}

/// Returns `url` with the value of every query parameter named in
/// `secret_query` replaced by [`MASK`]. Empty values stay empty (an empty
/// secret means "not entered yet"), everything else is kept byte for byte.
pub fn mask_url(url: &str, secret_query: &[String]) -> String {
    let (rest, fragment) = match url.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (url, None),
    };
    let Some((base, query)) = rest.split_once('?') else {
        return url.to_owned();
    };

    let masked: Vec<String> = query
        .split('&')
        .map(|pair| {
            let Some((raw_name, raw_value)) = pair.split_once('=') else {
                return pair.to_owned();
            };
            let name = url::form_urlencoded::parse(raw_name.as_bytes())
                .next()
                .map(|(name, _)| name.into_owned())
                .unwrap_or_default();
            if raw_value.is_empty() || !secret_query.contains(&name) {
                pair.to_owned()
            } else {
                format!("{raw_name}={MASK}")
            }
        })
        .collect();

    let mut out = format!("{base}?{}", masked.join("&"));
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}
