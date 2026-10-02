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
    /// Ordered exclude conditions.
    pub excludes: Vec<String>,
    /// Query names whose values are secret. Each must occur in `url`.
    pub secret_query: Vec<String>,
    /// Past-episode search template such as `[SubsPlease] {match} 1080p`.
    pub past_search: Option<String>,
    /// Display name. `None` means unnamed: the host is shown instead. The
    /// store trims it and treats a blank one as `None`.
    pub name: Option<String>,
}

impl ChannelInput {
    /// A new channel starts with every query value secret, so nothing leaks
    /// into an export before the user opts a value out.
    pub fn new(url: impl Into<String>) -> Self {
        let url = url.into();
        ChannelInput {
            secret_query: query_names(&url),
            url,
            excludes: Vec::new(),
            past_search: None,
            name: None,
        }
    }

    /// The name as stored: trimmed, and `None` when blank.
    pub(super) fn stored_name(&self) -> Option<&str> {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
    }

    pub(super) fn validate(&self) -> Result<(), ChannelError> {
        // `Url::parse` drops tabs and newlines that `mask_url` would still
        // see in the raw text, so such a URL could name a query differently
        // for the two and leave a secret unmasked.
        if self
            .url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(ChannelError::Invalid(
                "channel url contains whitespace or control characters",
            ));
        }
        let parsed = Url::parse(&self.url)
            .map_err(|_| ChannelError::Invalid("channel url is not a valid URL"))?;
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
            .field("excludes", &self.excludes)
            .field("secret_query", &self.secret_query)
            .field("past_search", &self.past_search)
            .field("name", &self.name)
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
    pub excludes: Vec<String>,
    pub secret_query: Vec<String>,
    pub past_search: Option<String>,
    /// Display name; `None` when the user left it blank.
    pub name: Option<String>,
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
            excludes: self.excludes.clone(),
            secret_query: self.secret_query.clone(),
            past_search: self.past_search.clone(),
            name: self.name.clone(),
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
            .field("excludes", &self.excludes)
            .field("secret_query", &self.secret_query)
            .field("past_search", &self.past_search)
            .field("name", &self.name)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleState {
    Active,
    /// `영상 받기` is off: the rule keeps its place and its work folder, and
    /// collects nothing until it is turned on again. Archiving (which moves
    /// the folder) is a different state.
    Paused,
    /// Kept in place but not collected; the work folder may be in the archive
    /// folder.
    Archived,
}

impl RuleState {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleState::Active => "active",
            RuleState::Paused => "paused",
            RuleState::Archived => "archived",
        }
    }

    pub(super) fn parse(s: &str) -> Option<RuleState> {
        match s {
            "active" => Some(RuleState::Active),
            "paused" => Some(RuleState::Paused),
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
    /// Save directory relative to the app's collect folder.
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
                "rule directory must be relative to the collect folder",
            ));
        }
        Ok(())
    }
}

/// How a subscription gets subtitles (`docs/specs/collection.md`, 방영작 구독).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleMode {
    /// Follow the subtitle creator named in the subscription.
    Follow,
    /// `제작자 미정`: no creator chosen yet.
    Undecided,
    /// `받지 않음`: video only.
    None,
}

impl SubtitleMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SubtitleMode::Follow => "follow",
            SubtitleMode::Undecided => "undecided",
            SubtitleMode::None => "none",
        }
    }

    pub fn parse(value: &str) -> Option<SubtitleMode> {
        match value {
            "follow" => Some(SubtitleMode::Follow),
            "undecided" => Some(SubtitleMode::Undecided),
            "none" => Some(SubtitleMode::None),
            _ => None,
        }
    }
}

/// The subscription of a rule: the rule follows an anime of Anissia's
/// schedule (`docs/specs/collection.md`, 방영작 구독). A rule without one is not
/// a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    /// Anissia's `animeNo`, confirmed when the rule was subscribed.
    pub anissia_anime_no: i64,
    pub subtitles: SubtitleMode,
    /// The subtitle creator: set when `subtitles` is [`SubtitleMode::Follow`],
    /// empty for [`SubtitleMode::Undecided`], and for [`SubtitleMode::None`]
    /// the creator followed before, if any, so switching back restores it.
    pub creator: Option<String>,
    /// The season the rule's videos belong to ([`SeasonRef::id`]); `None`
    /// until it is connected. Once set it stays.
    pub season_id: Option<String>,
    /// The season the rule's videos are in when another Anissia anime holds it
    /// already, so the rule could not be connected to it. `None` otherwise.
    pub season_blocked: Option<String>,
    /// When the rule became a subscription (Unix ms). Items the feed held and
    /// history had recorded before are past: the user receives those.
    pub subscribed_at: i64,
    /// When the subscription's match phrase was last given or cleared (Unix ms):
    /// a subscription that waited for its title was given one, or one that had a
    /// phrase was blanked to wait again. `None` for one that had its phrase from
    /// the start. What history first saw before then is past too.
    pub titled_at: Option<i64>,
}

/// A season of a work in the library: what a subscription connects to. The ID
/// kept in `season_id` is `<work id>:<season number>`; a season has no ID of
/// its own in the library, which keys its seasons by work and number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonRef {
    pub work_id: String,
    pub number: u32,
}

impl SeasonRef {
    pub fn id(&self) -> String {
        format!("{}:{}", self.work_id, self.number)
    }

    /// The season an ID from [`SeasonRef::id`] names.
    pub fn parse(id: &str) -> Option<SeasonRef> {
        let (work_id, number) = id.rsplit_once(':')?;
        if work_id.is_empty() {
            return None;
        }
        Some(SeasonRef {
            work_id: work_id.to_owned(),
            number: number.parse().ok()?,
        })
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
    /// Set when the rule follows an anime of Anissia's schedule.
    pub subscription: Option<Subscription>,
    /// When the rule was last turned back on (`영상 받기` on, or restored), as
    /// Unix milliseconds; `None` while it never was. What history first saw
    /// before then is left to the user ([`crate::worker::plan::ChannelPlan::is_past`]).
    pub resumed_at: Option<i64>,
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
