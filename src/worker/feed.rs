//! Reading a channel's RSS feed.

use std::time::Duration;

use reqwest::header;

use crate::{
    store::history::{identity_key, stored_link},
    transmission::Redactor,
};

/// How long one feed request may take in total. The legacy binary had no
/// limit, which a process that never exits cannot afford.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(60);

/// A failure to read a feed. Its text never contains the request URL, because
/// the URL carries the channel's secret query values.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("request failed: {0}")]
    Http(reqwest::Error),
    #[error("HTTP status {0}")]
    Status(u16),
    #[error("not a valid RSS feed: {0}")]
    Parse(rss::Error),
}

pub fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(crate::USER_AGENT)
        .timeout(FETCH_TIMEOUT)
        .build()
}

pub async fn fetch(client: &reqwest::Client, url: &str) -> Result<rss::Channel, FetchError> {
    let response = client
        .get(url)
        .header(header::USER_AGENT, crate::USER_AGENT)
        .send()
        .await
        .map_err(|e| FetchError::Http(e.without_url()))?;

    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::Status(status.as_u16()));
    }

    let body = response
        .bytes()
        .await
        .map_err(|e| FetchError::Http(e.without_url()))?;

    rss::Channel::read_from(&body[..]).map_err(FetchError::Parse)
}

/// One RSS item as the worker sees it.
///
/// `title` and `link` are the feed's own text: the title is judged as it is, and
/// the link is what Transmission is asked to add, so both may hold a secret and
/// are never stored or logged. `stored_title` and `stored_link` are their
/// counterparts for history and logs.
#[derive(Clone, PartialEq, Eq)]
pub struct FeedItem {
    /// See [`identity_key`].
    pub identity_key: String,
    /// Empty when the feed gives none; judged like any other title.
    pub title: String,
    /// The title with the channel's secret values replaced.
    pub stored_title: String,
    /// The link as given.
    pub link: String,
    /// The link with the values of the channel's secret query names masked and
    /// its secret values replaced wherever they occur (another parameter name,
    /// the path, a percent-encoded `tr=` of a magnet link).
    pub stored_link: String,
}

impl std::fmt::Debug for FeedItem {
    /// Leaves out the raw `title` and `link`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeedItem")
            .field("identity_key", &self.identity_key)
            .field("stored_title", &self.stored_title)
            .field("stored_link", &self.stored_link)
            .finish_non_exhaustive()
    }
}

/// The feed's items in feed order. An item whose identity key already appeared
/// earlier in the same feed (the same GUID, or the same link or title when
/// there is no GUID) is dropped, so one sighting produces one record. Items are
/// never dropped for looking alike after masking: the key is built from the
/// unmasked value.
///
/// `redactor` knows the channel's secret values; the texts meant for history
/// and logs go through it (the link after the name-based masking).
pub fn items(
    channel: &rss::Channel,
    secret_query: &[String],
    redactor: &Redactor,
) -> Vec<FeedItem> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for item in channel.items() {
        let key = identity_key(item.guid().map(|g| g.value()), item.link(), item.title());
        if !seen.insert(key.clone()) {
            continue;
        }
        let title = item.title().unwrap_or_default();
        out.push(FeedItem {
            identity_key: key,
            title: title.to_owned(),
            stored_title: redactor.apply(title),
            link: item.link().unwrap_or_default().to_owned(),
            stored_link: redactor.apply(&stored_link(item.link(), secret_query)),
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(items: &str) -> rss::Channel {
        let xml = format!(
            r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
            <link>http://x/</link><description>d</description>{items}</channel></rss>"#
        );
        rss::Channel::read_from(xml.as_bytes()).unwrap()
    }

    #[test]
    fn guid_wins_over_link_and_link_over_title() {
        let ch = feed(
            r#"<item><title>A</title><link>http://x/a</link><guid>g-a</guid></item>
               <item><title>B</title><link>http://x/b</link></item>
               <item><title>C</title></item>"#,
        );
        let keys: Vec<_> = items(&ch, &[], &Redactor::none())
            .into_iter()
            .map(|i| i.identity_key)
            .collect();
        assert_eq!(
            keys,
            [
                identity_key(Some("g-a"), None, None),
                identity_key(None, Some("http://x/b"), None),
                identity_key(None, None, Some("C")),
            ]
        );
        assert!(keys[0].starts_with("guid:") && keys[1].starts_with("link:"));
        assert!(keys[2].starts_with("title:"));
    }

    #[test]
    fn the_same_key_twice_in_one_feed_is_one_item() {
        let ch = feed(
            r#"<item><title>A</title><guid>same</guid></item>
               <item><title>A again</title><guid>same</guid></item>
               <item><title>B</title><guid>other</guid></item>"#,
        );
        let got = items(&ch, &[], &Redactor::none());
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].title, "A");
    }

    #[test]
    fn stored_link_is_masked_but_the_link_for_transmission_is_not() {
        let ch = feed(
            r#"<item><title>A</title><link>https://t.test/dl?id=1&amp;passkey=abc123</link></item>"#,
        );
        let got = items(&ch, &["passkey".to_owned()], &Redactor::none());
        assert!(got[0].link.contains("abc123"));
        assert!(!got[0].stored_link.contains("abc123"));
        assert!(!got[0].identity_key.contains("abc123"));
        assert!(!got[0].identity_key.contains("t.test"));
    }

    #[test]
    fn secret_values_are_replaced_wherever_they_occur_in_stored_texts() {
        // As the channel URL spells the value; the decoded form is `TOKEN`.
        const IN_URL: &str = "Tk%2Fen%2B0123456789";
        const TOKEN: &str = "Tk/en+0123456789";
        let mut redactor = Redactor::none();
        redactor.add_query_value(IN_URL);
        let encoded_twice = "Tk%252Fen%252B0123456789"; // inside a magnet tr=
        let ch = feed(&format!(
            r#"<item><title>Show [{TOKEN}] - 01</title>
                 <link>magnet:?xt=urn:btih:AAAA&amp;tr=https%3A%2F%2Ftr.test%2Fa%3Fpasskey%3D{encoded_twice}</link>
                 <guid>https://t.test/{TOKEN}/1</guid></item>
               <item><title>Other</title>
                 <link>https://t.test/dl?torrent_pass={TOKEN}&amp;id=3</link></item>"#
        ));

        let got = items(&ch, &[], &redactor);
        for item in &got {
            for text in [&item.stored_title, &item.stored_link, &item.identity_key] {
                assert!(!text.contains("0123456789"), "{text}");
                assert!(!text.contains(TOKEN), "{text}");
            }
        }
        assert_eq!(got[0].stored_title, "Show [***] - 01");
        assert!(got[0]
            .stored_link
            .contains("tr=https%3A%2F%2Ftr.test%2Fa%3Fpasskey%3D***"));
        assert_eq!(
            got[1].stored_link,
            "https://t.test/dl?torrent_pass=***&id=3"
        );
        // What Transmission is asked to add and what is judged stay as the feed gave them.
        assert!(got[0].link.contains(encoded_twice));
        assert!(got[0].title.contains(TOKEN));
        // Debug leaves them out.
        let shown = format!("{:?}", got[0]);
        assert!(!shown.contains("0123456789"), "{shown}");
    }

    #[test]
    fn items_without_title_or_link_still_get_a_key() {
        let ch = feed(r#"<item><description>x</description></item>"#);
        let got = items(&ch, &[], &Redactor::none());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "");
        assert_eq!(got[0].identity_key, identity_key(None, None, None));
    }

    #[test]
    fn items_that_differ_only_in_a_secret_query_value_are_both_kept() {
        // Every query name is secret by default; that must not merge items.
        let ch = feed(
            r#"<item><title>A</title><guid>https://t.test/details.php?id=101</guid></item>
               <item><title>B</title><guid>https://t.test/details.php?id=102</guid></item>
               <item><title>C</title><link>https://t.test/dl?id=1&amp;token=x</link></item>
               <item><title>D</title><link>https://t.test/dl?id=1&amp;token=y</link></item>"#,
        );
        let got = items(
            &ch,
            &["id".to_owned(), "token".to_owned()],
            &Redactor::none(),
        );
        let titles: Vec<_> = got.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["A", "B", "C", "D"]);
        let keys: std::collections::HashSet<_> = got.iter().map(|i| &i.identity_key).collect();
        assert_eq!(keys.len(), 4);
    }
}
