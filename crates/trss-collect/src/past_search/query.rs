//! The words of a search and the address it is sent to.
//!
//! The first search is the channel's search format with the rule's match
//! phrase in it (`[SubsPlease] {match} 1080p`), or, for a channel without a
//! format, the phrase itself, which the person edits for this one search. An
//! extra search of a long series keeps those words and writes the episodes as
//! the first results wrote them ([`Notation::alternatives`]).

use regex::Regex;
use url::Url;

use crate::{past_search::release::Notation, store::channels::Rule};

/// The place in a search format that the rule's match phrase takes.
pub const PLACEHOLDER: &str = "{match}";

/// What the search box starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefill {
    pub query: String,
    /// Whether the channel's search format made it.
    pub from_format: bool,
    /// Why the box starts empty, when it does.
    pub empty_because: Option<&'static str>,
}

/// Runs of whitespace become one space, and the ends are trimmed.
pub fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The search a rule starts with in a channel with this search `format`.
pub fn prefill(format: Option<&str>, rule: &Rule) -> Prefill {
    let Some(phrase) = rule.r#match.as_deref().filter(|m| !m.trim().is_empty()) else {
        return Prefill {
            query: String::new(),
            from_format: false,
            empty_because: Some("제목이 정해지지 않은 규칙이라 검색어를 만들 수 없어요."),
        };
    };
    if rule.regex {
        return Prefill {
            query: String::new(),
            from_format: false,
            empty_because: Some(
                "정규식 규칙이라 일치 문구를 검색어로 쓸 수 없어요. 검색어를 직접 적어 주세요.",
            ),
        };
    }
    match format.map(str::trim).filter(|f| !f.is_empty()) {
        Some(format) => Prefill {
            query: tidy(&format.replace(PLACEHOLDER, phrase.trim())),
            from_format: true,
            empty_because: None,
        },
        None => Prefill {
            query: tidy(phrase),
            from_format: false,
            empty_because: None,
        },
    }
}

/// The search for the episodes `numbers`: `query` with the first occurrence of
/// the rule's match phrase followed by the episodes in `notation`. `None` when
/// the query does not hold the phrase, so there is no telling where the
/// episodes go.
pub fn batch_query(
    query: &str,
    phrase: &str,
    notation: Notation,
    numbers: &[u32],
) -> Option<String> {
    let phrase = tidy(phrase);
    if phrase.is_empty() || numbers.is_empty() {
        return None;
    }
    let pattern = Regex::new(&format!("(?i){}", regex::escape(&phrase))).ok()?;
    let found = pattern.find(query)?;
    let mut out = String::with_capacity(query.len() + 32);
    out.push_str(&query[..found.end()]);
    out.push_str(&notation.alternatives(numbers));
    out.push_str(&query[found.end()..]);
    Some(tidy(&out))
}

/// The address of a search in the channel `url`: the channel's own address with
/// the search words as `q` and without a page number (`p`), so the other
/// conditions of the channel (category, filter, uploader) still apply. `None`
/// for an address that is not a URL.
pub fn search_url(channel_url: &str, query: &str) -> Option<String> {
    let mut url = Url::parse(channel_url).ok()?;
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(name, _)| name != "q" && name != "p")
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    {
        let mut pairs = url.query_pairs_mut();
        pairs.extend_pairs(kept.iter().map(|(n, v)| (n.as_str(), v.as_str())));
        pairs.append_pair("q", query);
    }
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::channels::RuleState;

    fn rule(phrase: Option<&str>, regex: bool) -> Rule {
        Rule {
            id: "r1".into(),
            channel_id: "c1".into(),
            position: 0,
            version: 1,
            r#match: phrase.map(str::to_owned),
            regex,
            case_insensitive: false,
            directory: "Show/Season 01".into(),
            episode: 1,
            episode_auto: false,
            state: RuleState::Active,
            subscription: None,
            resumed_at: None,
        }
    }

    #[test]
    fn the_format_takes_the_match_phrase() {
        let start = prefill(
            Some("[SubsPlease] {match} 1080p"),
            &rule(Some("Sayonara Lara"), false),
        );
        assert_eq!(start.query, "[SubsPlease] Sayonara Lara 1080p");
        assert!(start.from_format);
        assert_eq!(start.empty_because, None);
    }

    #[test]
    fn a_channel_without_a_format_starts_with_the_phrase() {
        for format in [None, Some(""), Some("   ")] {
            let start = prefill(format, &rule(Some("[SubsPlease] Sayonara  Lara"), false));
            assert_eq!(start.query, "[SubsPlease] Sayonara Lara");
            assert!(!start.from_format);
        }
    }

    #[test]
    fn a_format_without_the_placeholder_is_used_as_written() {
        let start = prefill(Some("Sayonara Lara 1080p"), &rule(Some("anything"), false));
        assert_eq!(start.query, "Sayonara Lara 1080p");
    }

    #[test]
    fn a_rule_without_a_phrase_or_with_a_regex_starts_empty_and_says_why() {
        let waiting = prefill(Some("{match}"), &rule(None, false));
        assert_eq!(waiting.query, "");
        assert!(waiting.empty_because.is_some());
        let regex = prefill(Some("{match}"), &rule(Some("^Show - \\d+"), true));
        assert_eq!(regex.query, "");
        assert!(regex.empty_because.is_some());
    }

    #[test]
    fn a_batch_search_writes_the_episodes_after_the_phrase() {
        let notation = Notation::Dash { width: 4 };
        assert_eq!(
            batch_query(
                "[SubsPlease] One Piece 1080p",
                "One Piece",
                notation,
                &[1000, 1001, 1002]
            ),
            Some("[SubsPlease] One Piece - (1000|1001|1002) 1080p".to_owned())
        );
        // The phrase may be the whole query, in any case.
        assert_eq!(
            batch_query("one piece", "One Piece", notation, &[1000]),
            Some("one piece - (1000)".to_owned())
        );
        assert_eq!(
            batch_query("Other Show", "One Piece", notation, &[1000]),
            None
        );
        assert_eq!(batch_query("One Piece", "One Piece", notation, &[]), None);
    }

    #[test]
    fn the_search_address_keeps_the_channels_conditions() {
        let url = search_url(
            "https://nyaa.si/?page=rss&q=old&c=1_2&f=0&p=3&u=SubsPlease",
            "[SubsPlease] One Piece - (1000|1001)",
        )
        .unwrap();
        let parsed = Url::parse(&url).unwrap();
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(n, v)| (n.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("page".into(), "rss".into()),
                ("c".into(), "1_2".into()),
                ("f".into(), "0".into()),
                ("u".into(), "SubsPlease".into()),
                ("q".into(), "[SubsPlease] One Piece - (1000|1001)".into()),
            ]
        );
        assert_eq!(parsed.host_str(), Some("nyaa.si"));
        assert_eq!(parsed.path(), "/");
    }

    #[test]
    fn a_channel_without_a_query_gets_only_the_search() {
        let url = search_url("https://tracker.test/rss", "Show").unwrap();
        assert_eq!(url, "https://tracker.test/rss?q=Show");
        assert_eq!(search_url("not a url", "Show"), None);
    }
}
