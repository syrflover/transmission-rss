//! Reading the legacy channels YAML.
//!
//! The file is a list of channels (`url`, `directory`, optional `excludes`,
//! `rules`), each rule having `match`, `directory` and optional `regex`,
//! `case_insensitive` and `episode` (default 1). The deserialization is the
//! one the old executable used ([`ChannelConfig`]); this module adds the checks
//! the app database needs and turns failures into Korean sentences that say why
//! the file cannot be imported.
//!
//! The channel `directory` is not stored on the channel any more (the app has
//! one collect folder); [`LegacyChannel`] carries it so [`super::fit`] can
//! place the channel under the collect folder.
//!
//! An error message never repeats text from the file, because the file holds
//! feed tokens.

use std::fmt;

use url::Url;
use yaml_serde::Value;

use super::comments::{rule_comments, Reading};
use crate::config::ChannelConfig;
use crate::store::channels::import::ImportChannel;
use crate::store::channels::{ChannelInput, RuleInput, RuleState};

/// Why a file cannot be imported. The message is a Korean sentence for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(String);

impl ParseError {
    fn new(message: impl Into<String>) -> Self {
        ParseError(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// One channel of the file: the folder its `directory` names, and the channel
/// with its rules. The rules' `directory` is relative to [`Self::folder`] until
/// [`super::fit::fit`] puts it under the collect folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyChannel {
    pub folder: String,
    pub channel: ImportChannel,
    /// What the comment above each rule says, one per rule of
    /// [`Self::channel`] in order ([`super::comments`]).
    pub readings: Vec<Reading>,
}

/// Parses the file into channels in file order, each with its rules in file
/// order. Every query name of every URL is secret. A rule with `match: ""`
/// is refused: the old executable matched every title with it, while the app
/// has no match-everything rule and reads an empty phrase as waiting for a
/// title, so neither reading would keep what the file meant.
pub fn parse(content: &str) -> Result<Vec<LegacyChannel>, ParseError> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);

    let document: Value = yaml_serde::from_str(content).map_err(|e| {
        ParseError::new(match e.location() {
            Some(at) => format!(
                "YAML로 읽을 수 없어요. {}번째 줄 근처의 서식을 확인해 주세요.",
                at.line()
            ),
            None => "YAML로 읽을 수 없어요. 파일이 YAML 서식인지 확인해 주세요.".to_owned(),
        })
    })?;

    let channels = match &document {
        Value::Null => return Err(ParseError::new("파일이 비어 있어요.")),
        Value::Sequence(channels) if channels.is_empty() => {
            return Err(ParseError::new("파일에 가져올 채널이 없어요."))
        }
        Value::Sequence(channels) => channels,
        _ => {
            return Err(ParseError::new(
                "채널 목록이 아니에요. 기존 설정 파일은 `- url:`로 시작하는 채널 목록이에요.",
            ))
        }
    };

    for (i, channel) in channels.iter().enumerate() {
        check_present(channel, i, &["url", "directory", "rules"], |n, key| {
            format!("{n}번째 채널에 `{key}`가 없어요.")
        })?;
        let rules = channel.get("rules").and_then(Value::as_sequence);
        for (j, rule) in rules.into_iter().flatten().enumerate() {
            check_present(rule, j, &["match", "directory"], |n, key| {
                format!("{}번째 채널의 {n}번째 규칙에 `{key}`가 없어요.", i + 1)
            })?;
        }
    }

    // The shape is right; the same deserialization as the old executable
    // settles the value types and fills the omitted defaults.
    let configs: Vec<ChannelConfig> = yaml_serde::from_str(content).map_err(|e| {
        let line = e
            .location()
            .map(|at| format!(" {}번째 줄 근처를", at.line()))
            .unwrap_or_default();
        ParseError::new(format!(
            "값의 형식이 맞지 않아요.{line} 확인해 주세요. `regex`와 `case_insensitive`는 true나 false, `episode`는 정수, 나머지는 글자여야 해요."
        ))
    })?;

    let mut channels = configs
        .iter()
        .enumerate()
        .map(|(i, config)| convert(i, config))
        .collect::<Result<Vec<_>, _>>()?;

    // The comments are read from the text: the parser above drops them.
    let counts: Vec<usize> = channels.iter().map(|c| c.channel.rules.len()).collect();
    for (channel, readings) in channels.iter_mut().zip(rule_comments(content, &counts)) {
        channel.readings = readings;
    }
    Ok(channels)
}

/// Ensures the `index`-th item is a mapping holding each of `keys`.
fn check_present(
    item: &Value,
    index: usize,
    keys: &[&str],
    missing: impl Fn(usize, &str) -> String,
) -> Result<(), ParseError> {
    let n = index + 1;
    if item.as_mapping().is_none() {
        return Err(ParseError::new(format!(
            "{n}번째 항목이 설정이 아니에요. `{}` 같은 항목을 가진 묶음이어야 해요.",
            keys.join("`, `")
        )));
    }
    match keys.iter().find(|key| item.get(**key).is_none()) {
        Some(key) => Err(ParseError::new(missing(n, key))),
        None => Ok(()),
    }
}

fn convert(index: usize, config: &ChannelConfig) -> Result<LegacyChannel, ParseError> {
    let n = index + 1;

    let url = config.url.trim();
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(ParseError::new(format!(
            "{n}번째 채널의 `url`에 공백이나 줄바꿈이 들어 있어요."
        )));
    }
    match Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => {}
        _ => {
            return Err(ParseError::new(format!(
                "{n}번째 채널의 `url`이 http나 https 주소가 아니에요."
            )))
        }
    }

    let folder = config.directory.to_string_lossy().into_owned();
    if folder.is_empty() {
        return Err(ParseError::new(format!(
            "{n}번째 채널의 `directory`가 비어 있어요."
        )));
    }

    let mut rules = Vec::with_capacity(config.rules.len());
    for (j, rule) in config.rules.iter().enumerate() {
        if rule.directory.is_absolute() {
            return Err(ParseError::new(format!(
                "{n}번째 채널의 {}번째 규칙 `directory`가 절대 경로예요. 채널 `directory` 아래의 상대 경로여야 해요.",
                j + 1
            )));
        }
        if rule.r#match.is_empty() {
            return Err(ParseError::new(format!(
                "{n}번째 채널의 {}번째 규칙은 `match`가 비어 있어요. 지금 실행 파일에서는 모든 항목에 맞는 규칙이지만 앱에는 그런 규칙이 없어요. 일치 문구를 채우거나 규칙을 지운 뒤 다시 가져와 주세요.",
                j + 1
            )));
        }
        rules.push(RuleInput {
            r#match: Some(rule.r#match.clone()),
            regex: rule.regex,
            case_insensitive: rule.case_insensitive,
            directory: rule.directory.to_string_lossy().into_owned(),
            episode: rule.starts_episode_at as i64,
            episode_auto: false,
            state: RuleState::Active,
        });
    }

    let mut input = ChannelInput::new(url);
    input.excludes = config.excludes.clone();
    Ok(LegacyChannel {
        folder,
        channel: ImportChannel { input, rules },
        readings: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(content: &str) -> String {
        parse(content).unwrap_err().message().to_owned()
    }

    #[test]
    fn reads_the_sample_in_file_order_with_every_value() {
        let sample = include_str!("../../tests/fixtures/legacy_channels.yml");
        let channels = parse(sample).unwrap();
        assert_eq!(channels.len(), 2);

        assert_eq!(channels[0].folder, "/media/anime");
        assert_eq!(channels[1].folder, "/media/other");
        let first = &channels[0].channel;
        assert_eq!(
            first.input.url,
            "https://feeds.example.test/subsplease?filter=1080p&token=REDACTED"
        );
        assert_eq!(first.input.excludes, ["[Batch]", "(720p)"]);
        assert_eq!(first.input.secret_query, ["filter", "token"]);
        let phrases: Vec<_> = first
            .rules
            .iter()
            .map(|r| r.r#match.as_deref().unwrap())
            .collect();
        assert_eq!(
            phrases,
            [
                "[SubsPlease] Sayonara Lara - ",
                "sono bisque doll",
                "Sono Bisque Doll",
                "[SubsPlease] Tensei Shitara Slime Datta Ken",
                "Lowercase Show",
            ]
        );
        // Omitted values take the old defaults; given ones are kept as written.
        assert_eq!(first.rules[0].episode, 1);
        assert!(!first.rules[0].regex && !first.rules[0].case_insensitive);
        assert_eq!(first.rules[1].episode, -12);
        assert!(first.rules[1].case_insensitive);
        assert_eq!(first.rules[3].episode, -24);
        assert_eq!(first.rules[3].directory, "Slime/Season 04");
        assert_eq!(channels[1].channel.input.excludes, Vec::<String>::new());
        assert_eq!(channels[1].channel.rules.len(), 2);
    }

    #[test]
    fn the_comments_above_the_rules_are_read_alongside_each_rule() {
        let sample = include_str!("../../tests/fixtures/legacy_commented.yml");
        let channels = parse(sample).unwrap();
        let counts: Vec<_> = channels.iter().map(|c| c.readings.len()).collect();
        assert_eq!(counts, [5, 2]);
        assert_eq!(
            channels[0].readings[0],
            Reading::Address {
                anime_no: 1001,
                creator: Some("Team Alpha".into()),
                airs: Some(crate::import::comments::Airs {
                    week: 3,
                    time: "22:30".into()
                })
            }
        );
        assert_eq!(channels[0].readings[3], Reading::None);
        // The rules themselves are the same with and without the comments.
        assert_eq!(channels[0].channel.rules[3].episode, -12);
        assert_eq!(
            channels[1].channel.rules[1].directory,
            "Zeta Show/Season 01"
        );

        // A file with no comments has none to read.
        let plain = parse(include_str!("../../tests/fixtures/legacy_channels.yml")).unwrap();
        assert!(plain
            .iter()
            .flat_map(|c| &c.readings)
            .all(|r| *r == Reading::None));
    }

    #[test]
    fn keeps_negative_episodes_and_regex_flags_exactly() {
        let channels = parse(
            "- url: https://x.test/rss\n  directory: /m\n  rules:\n    - match: '^Show \\d+$'\n      regex: true\n      case_insensitive: true\n      episode: -24\n      directory: Show\n",
        )
        .unwrap();
        let rule = &channels[0].channel.rules[0];
        assert_eq!(rule.r#match.as_deref(), Some("^Show \\d+$"));
        assert!(rule.regex && rule.case_insensitive);
        assert_eq!(rule.episode, -24);
    }

    #[test]
    fn an_empty_match_is_refused_naming_the_rule() {
        let message = err(
            "- url: https://x.test/rss\n  directory: /m\n  rules:\n    - match: a\n      directory: A\n    - match: ''\n      directory: Later\n",
        );
        assert!(message.contains("1번째 채널의 2번째 규칙"), "{message}");
        assert!(message.contains("`match`가 비어"), "{message}");
    }

    #[test]
    fn rejects_files_that_are_not_channel_lists_with_a_reason() {
        assert!(err("").contains("비어"));
        assert!(err("# only a comment\n").contains("비어"));
        assert!(err("[]").contains("채널이 없"));
        assert!(err("just some text").contains("채널 목록이 아니"));
        assert!(err("key: value").contains("채널 목록이 아니"));
        assert!(err("- url: [unclosed").contains("YAML로 읽을 수 없"));
        assert!(err("- just a string").contains("1번째 항목"));
        assert!(err("\u{1}binary: [").contains("YAML"));
    }

    #[test]
    fn rejects_missing_required_fields_naming_the_field_and_place() {
        let base = "- url: https://x.test/rss\n  directory: /m\n  rules: []\n";
        assert!(err("- directory: /m\n  rules: []\n").contains("1번째 채널에 `url`"));
        assert!(err("- url: https://x.test/rss\n  rules: []\n").contains("`directory`"));
        assert!(err("- url: https://x.test/rss\n  directory: /m\n").contains("`rules`"));
        let second = format!("{base}- url: https://y.test/rss\n  directory: /m\n");
        assert!(err(&second).contains("2번째 채널에 `rules`"));
        let no_match = format!(
            "{base}- url: https://y.test/rss\n  directory: /m\n  rules:\n    - directory: d\n"
        );
        assert!(err(&no_match).contains("2번째 채널의 1번째 규칙에 `match`"));
        let no_dir = "- url: https://y.test/rss\n  directory: /m\n  rules:\n    - match: a\n";
        assert!(err(no_dir).contains("1번째 채널의 1번째 규칙에 `directory`"));
    }

    #[test]
    fn rejects_bad_values_without_repeating_file_text() {
        let secret = "s3cr3t-token";
        let bad_episode = format!(
            "- url: https://x.test/rss?token={secret}\n  directory: /m\n  rules:\n    - match: a\n      directory: d\n      episode: {secret}\n"
        );
        let message = err(&bad_episode);
        assert!(message.contains("값의 형식"), "{message}");
        assert!(!message.contains(secret));

        let bad_url = format!("- url: not-a-url-{secret}\n  directory: /m\n  rules: []\n");
        let message = err(&bad_url);
        assert!(message.contains("1번째 채널의 `url`"), "{message}");
        assert!(!message.contains(secret));

        assert!(err("- url: ftp://x.test/rss\n  directory: /m\n  rules: []\n").contains("http"));
        assert!(err("- url: https://x.test/rss\n  directory: ''\n  rules: []\n").contains("비어"));
        let absolute = "- url: https://x.test/rss\n  directory: /m\n  rules:\n    - match: a\n      directory: /abs\n";
        assert!(err(absolute).contains("절대 경로"));
    }

    #[test]
    fn a_byte_order_mark_is_ignored() {
        let content = format!(
            "\u{feff}{}",
            "- url: https://x.test/rss\n  directory: /m\n  rules: []\n"
        );
        assert_eq!(parse(&content).unwrap().len(), 1);
    }
}
