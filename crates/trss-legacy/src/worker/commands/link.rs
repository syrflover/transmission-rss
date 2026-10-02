//! Getting an item's original link back for a one-off receive.
//!
//! History keeps an item's link with the channel's secret values masked
//! ([`crate::store::history::stored_link`]), so it cannot be handed to
//! Transmission as it is. [`recover`] rebuilds the link in the order the
//! collection spec sets (수집 이력):
//!
//! 1. When the link's host is the channel URL's host (compared without regard
//!    to case; a magnet link has none), fill each masked query value from the
//!    value the channel's own URL stores under the same query name
//!    ([`fill_masked_values`]). A link on another host is never filled, so the
//!    channel's secret does not go to a host it was not made for. When the
//!    item's identity is its link, the result must hash to that identity, so
//!    a link whose secret differs from the channel's is not trusted.
//! 2. If a mask remains (the link is on another host, or the secret sits under
//!    another name, or in the path or a magnet's tracker), read the channel's
//!    feed now and take the raw link of the item with the same identity key.
//! 3. Otherwise give up with a reason that says the original link could not be
//!    recovered.
//!
//! The recovered link is the secret: it is returned to the caller for
//! Transmission only and never appears in a reason, an error or a log line.

use std::collections::HashMap;

use crate::{
    store::{
        channels::{Channel, MASK},
        history::{identity_key, HistoryItem},
    },
    worker::feed,
};
use trss_transmission::Redactor;

/// A link that could be rebuilt, or the sentence saying why not.
pub type Recovery = Result<String, String>;

/// The reason when neither the channel's stored values nor its feed give the link.
const NOT_IN_FEED: &str = "원래 링크를 되살리지 못했어요. 링크의 비밀 값을 채널에 저장된 값으로 채울 수 없고, 채널의 지금 RSS에도 이 항목이 없어요.";
/// The reason when the feed could not be read to look for the link.
const FEED_UNREADABLE: &str = "원래 링크를 되살리지 못했어요. 링크의 비밀 값을 채널에 저장된 값으로 채울 수 없고, 채널의 RSS도 읽지 못했어요.";

/// Recovers the raw link of `item` (see the module docs). `redactor` cleans
/// the feed error that goes into the reason.
pub async fn recover(
    item: &HistoryItem,
    channel: &Channel,
    http: &reqwest::Client,
    redactor: &Redactor,
) -> Recovery {
    if !has_mask(&item.link) {
        return Ok(item.link.clone());
    }

    if same_host(&item.link, &channel.url) {
        let filled = fill_masked_values(&item.link, &channel.url);
        if !has_mask(&filled) && matches_identity(&filled, &item.identity_key) {
            return Ok(filled);
        }
    }

    let read = feed::fetch(http, &channel.url).await.map_err(|err| {
        eprintln!(
            "Cannot read the feed of {} to recover a link: {}",
            channel.masked_url(),
            redactor.apply(&err.to_string())
        );
    });
    let Ok(read) = read else {
        return Err(FEED_UNREADABLE.to_owned());
    };
    feed::items(&read, &channel.secret_query, redactor)
        .into_iter()
        .find(|found| found.identity_key == item.identity_key && !found.link.trim().is_empty())
        .map(|found| found.link)
        .ok_or_else(|| NOT_IN_FEED.to_owned())
}

/// Whether some part of `link` is still masked.
pub fn has_mask(link: &str) -> bool {
    link.contains(MASK)
}

/// Whether both URLs name the same host, ignoring case. A URL without a host
/// (a magnet link, or text that is not a URL) matches nothing.
fn same_host(link: &str, channel_url: &str) -> bool {
    let host = |url: &str| {
        url::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
    };
    matches!((host(link), host(channel_url)), (Some(a), Some(b)) if a == b)
}

/// A link built from the feed item's link can be checked against the item's
/// identity key; with a GUID- or title-based key there is nothing to check.
fn matches_identity(link: &str, key: &str) -> bool {
    !key.starts_with("link:") || identity_key(None, Some(link), None) == key
}

/// Puts the channel's stored value back for every masked query value of
/// `stored_link` whose name the channel's URL has a value for. Repeated names
/// match by position: the second `t=***` takes the second `t=` of the channel
/// URL. Everything else is left byte for byte, so a mask that has no stored
/// value to take stays.
pub fn fill_masked_values(stored_link: &str, channel_url: &str) -> String {
    let (rest, fragment) = split_fragment(stored_link);
    let Some((base, query)) = rest.split_once('?') else {
        return stored_link.to_owned();
    };

    let mut stored: HashMap<String, Vec<&str>> = HashMap::new();
    let channel_rest = split_fragment(channel_url).0;
    if let Some((_, channel_query)) = channel_rest.split_once('?') {
        for pair in channel_query.split('&') {
            if let Some((raw_name, raw_value)) = pair.split_once('=') {
                stored.entry(decode(raw_name)).or_default().push(raw_value);
            }
        }
    }

    let mut seen: HashMap<String, usize> = HashMap::new();
    let pairs: Vec<String> = query
        .split('&')
        .map(|pair| {
            let Some((raw_name, raw_value)) = pair.split_once('=') else {
                return pair.to_owned();
            };
            let name = decode(raw_name);
            let position = {
                let count = seen.entry(name.clone()).or_default();
                *count += 1;
                *count - 1
            };
            if raw_value != MASK {
                return pair.to_owned();
            }
            match stored.get(&name).and_then(|values| values.get(position)) {
                Some(value) if !value.is_empty() => format!("{raw_name}={value}"),
                _ => pair.to_owned(),
            }
        })
        .collect();

    let mut out = format!("{base}?{}", pairs.join("&"));
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

fn split_fragment(url: &str) -> (&str, Option<&str>) {
    match url.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (url, None),
    }
}

/// A query name as the URL standard reads it (percent-decoded, `+` as space).
fn decode(raw_name: &str) -> String {
    url::form_urlencoded::parse(raw_name.as_bytes())
        .next()
        .map(|(name, _)| name.into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNEL: &str = "http://feeds.test/rss?filter=1080p&token=TOKEN123456&sig=SIG654321";

    #[test]
    fn a_masked_value_takes_the_channels_value_of_the_same_name() {
        assert_eq!(
            fill_masked_values("http://x.test/dl?id=7&token=***", CHANNEL),
            "http://x.test/dl?id=7&token=TOKEN123456"
        );
        assert_eq!(
            fill_masked_values("http://x.test/dl?token=***&sig=***#frag", CHANNEL),
            "http://x.test/dl?token=TOKEN123456&sig=SIG654321#frag"
        );
    }

    #[test]
    fn a_mask_with_no_value_of_that_name_stays() {
        let filled = fill_masked_values("http://x.test/dl?passkey=***&token=***", CHANNEL);
        assert_eq!(filled, "http://x.test/dl?passkey=***&token=TOKEN123456");
        assert!(has_mask(&filled));
    }

    #[test]
    fn a_public_value_and_a_link_without_a_query_are_left_alone() {
        assert_eq!(
            fill_masked_values("http://x.test/dl?id=7", CHANNEL),
            "http://x.test/dl?id=7"
        );
        assert_eq!(
            fill_masked_values("http://x.test/dl/***", CHANNEL),
            "http://x.test/dl/***"
        );
    }

    #[test]
    fn repeated_names_match_by_position() {
        assert_eq!(
            fill_masked_values(
                "http://x.test/dl?t=***&t=***&t=***",
                "http://feeds.test/rss?t=AAAAAAAA&t=BBBBBBBB"
            ),
            "http://x.test/dl?t=AAAAAAAA&t=BBBBBBBB&t=***"
        );
    }

    #[test]
    fn names_are_compared_after_decoding() {
        assert_eq!(
            fill_masked_values(
                "http://x.test/dl?to%6Ben=***",
                "http://feeds.test/rss?token=TOKEN123456"
            ),
            "http://x.test/dl?to%6Ben=TOKEN123456"
        );
    }

    #[test]
    fn only_a_link_on_the_channels_host_is_filled() {
        assert!(same_host("http://FEEDS.test/dl?token=***", CHANNEL));
        assert!(same_host("https://feeds.test:8443/dl?token=***", CHANNEL));
        assert!(!same_host("http://elsewhere.test/dl?token=***", CHANNEL));
        assert!(!same_host("http://feeds.test.elsewhere.test/dl", CHANNEL));
        assert!(!same_host("magnet:?xt=urn:btih:abc&token=***", CHANNEL));
        assert!(!same_host("not a url", CHANNEL));
    }

    #[test]
    fn a_link_based_identity_must_match_the_filled_link() {
        let link = "http://x.test/dl?id=7&token=TOKEN123456";
        let key = identity_key(None, Some(link), None);
        assert!(matches_identity(link, &key));
        assert!(!matches_identity(
            "http://x.test/dl?id=7&token=OTHERTOKEN99",
            &key
        ));
        // A GUID- or title-based key says nothing about the link.
        assert!(matches_identity(
            link,
            &identity_key(Some("guid"), Some(link), None)
        ));
    }
}
