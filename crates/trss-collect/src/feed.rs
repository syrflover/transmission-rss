//! Reading a channel's RSS feed.

use std::time::Duration;

use reqwest::header;

use crate::store::history::{identity_key, stored_link};
use trss_transmission::Redactor;

/// How long one feed request may take in total. The former binary had no
/// limit, which a process that never exits cannot afford.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(60);

/// The most bytes of a feed body that are read. Real feeds are far smaller (a
/// tracker's RSS of 75 to 100 items is 50 to 300 KB), so 2 MiB leaves a wide
/// margin, as [`trss_anissia::MAX_ANSWER_BYTES`] does for its answers. The
/// worker container has 256M, most of it kept for unpacking an archive in a
/// child process; up to `FETCH_CONCURRENCY` feeds are held at once
/// and each is parsed into a structure several times its size, so a bigger cap
/// would let a few oversized or endless bodies exhaust it. The cap counts the
/// bytes after any content decoding, so a compressed bomb is stopped too.
pub const MAX_FEED_BYTES: usize = 2 * 1024 * 1024;

/// How long to wait when a `429` answer names no time.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);
/// The longest `Retry-After` honoured as given.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

/// A failure to read a feed. Its text never contains the request URL, because
/// the URL carries the channel's secret query values.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("request failed: {0}")]
    Http(reqwest::Error),
    #[error("HTTP status {0}")]
    Status(u16),
    /// `429`: the server asks for a pause, as long as `Retry-After` says (a
    /// minute when it names no time, an hour at most).
    #[error("the server asks to wait {}s", .0.as_secs())]
    Busy(Duration),
    #[error("the feed is larger than {MAX_FEED_BYTES} bytes")]
    TooLarge,
    #[error("not a valid RSS feed: {0}")]
    Parse(rss::Error),
}

/// The client for channel feeds and search pages. A redirect is followed as
/// reqwest does by default, but with no `Referer`: reqwest would put the
/// previous address there with its query, which holds the channel's secret
/// values, and send it to the next host.
pub fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(trss_core::USER_AGENT)
        .referer(false)
        .timeout(FETCH_TIMEOUT)
        .build()
}

pub async fn fetch(client: &reqwest::Client, url: &str) -> Result<rss::Channel, FetchError> {
    let response = client
        .get(url)
        .header(header::USER_AGENT, trss_core::USER_AGENT)
        .send()
        .await
        .map_err(|e| FetchError::Http(e.without_url()))?;

    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let wait = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map_or(DEFAULT_RETRY_AFTER, Duration::from_secs)
            .min(MAX_RETRY_AFTER);
        return Err(FetchError::Busy(wait));
    }
    if !status.is_success() {
        return Err(FetchError::Status(status.as_u16()));
    }

    let body = read_body(response).await?;

    rss::Channel::read_from(&body[..]).map_err(FetchError::Parse)
}

/// The body of a feed response, refused once it passes [`MAX_FEED_BYTES`]: at
/// once when the response announces a longer body, otherwise as soon as the
/// chunks read so far do. (The announced length is not trusted to be true, so
/// the chunks are counted either way.)
async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, FetchError> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_FEED_BYTES as u64)
    {
        return Err(FetchError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| FetchError::Http(e.without_url()))?
    {
        if body.len() + chunk.len() > MAX_FEED_BYTES {
            return Err(FetchError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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

/// A channel host that redirects to another host, for the tests of the
/// readers that use [`client`].
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, Mutex};

    use axum::{
        http::{header, HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
        Router,
    };

    /// The secret value the tests put in the channel URL's query.
    pub(crate) const SECRET: &str = "s3cr3tpasskey";

    /// The channel host (`127.0.0.1`) answers every request with a `302` to
    /// `/feed` on another host (`127.0.0.2`), which answers a feed of one item
    /// and keeps the headers of each request it gets.
    pub(crate) struct Redirect {
        /// `http://127.0.0.1:<port>`.
        pub(crate) base: String,
        seen: Arc<Mutex<Vec<HeaderMap>>>,
        tasks: [tokio::task::JoinHandle<()>; 2],
    }

    impl Redirect {
        pub(crate) async fn start() -> Redirect {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let other = tokio::net::TcpListener::bind("127.0.0.2:0").await.unwrap();
            let target = format!("http://{}/feed", other.local_addr().unwrap());
            let feed = Router::new().route(
                "/feed",
                get({
                    let seen = seen.clone();
                    move |headers: HeaderMap| async move {
                        seen.lock().unwrap().push(headers);
                        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
                        <link>http://x/</link><description>d</description>
                        <item><title>A</title><guid>a</guid></item></channel></rss>"#
                    }
                }),
            );
            let channel = Router::new().fallback(move || async move {
                (StatusCode::FOUND, [(header::LOCATION, target)]).into_response()
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let tasks = [
                tokio::spawn(async move {
                    axum::serve(other, feed).await.ok();
                }),
                tokio::spawn(async move {
                    axum::serve(listener, channel).await.ok();
                }),
            ];
            Redirect { base, seen, tasks }
        }

        /// Asserts that the other host was asked, and that no header of its
        /// requests carries [`SECRET`] or the channel host's address.
        pub(crate) fn assert_nothing_leaked(&self) {
            let seen = self.seen.lock().unwrap();
            assert!(!seen.is_empty(), "the redirect was not followed");
            let channel_host = self.base.trim_start_matches("http://");
            for headers in seen.iter() {
                for (name, value) in headers {
                    let value = String::from_utf8_lossy(value.as_bytes());
                    assert!(
                        !value.contains(SECRET) && !value.contains(channel_host),
                        "{name}: {value}"
                    );
                }
            }
        }
    }

    impl Drop for Redirect {
        fn drop(&mut self) {
            for task in &self.tasks {
                task.abort();
            }
        }
    }
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

    /// A feed server for the fetch tests: `/ok` is a normal feed, `/exact` a
    /// feed of exactly [`MAX_FEED_BYTES`], `/big` a feed followed by padding
    /// that makes it one byte over, streamed without a length, and `/declared`
    /// declares a length over the cap and then sends nothing more.
    async fn serve() -> (String, tokio::task::JoinHandle<()>) {
        use axum::{
            body::{Body, Bytes},
            http::header,
            response::IntoResponse,
            routing::get,
            Router,
        };

        const ITEM: &str = "<item><title>A</title><guid>a</guid></item>";

        fn padded(total: usize) -> Vec<u8> {
            let mut body = format!(
                r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
                <link>http://x/</link><description>d</description>{ITEM}</channel></rss><!--"#
            )
            .into_bytes();
            let rest = total - body.len() - "-->".len();
            body.extend(std::iter::repeat_n(b' ', rest));
            body.extend_from_slice(b"-->");
            assert_eq!(body.len(), total);
            body
        }
        fn chunks(body: Vec<u8>) -> Body {
            let parts: Vec<Result<Bytes, std::convert::Infallible>> = body
                .chunks(64 * 1024)
                .map(|c| Ok(Bytes::copy_from_slice(c)))
                .collect();
            Body::from_stream(futures::stream::iter(parts))
        }

        let app = Router::new()
            .route("/ok", get(|| async { chunks(padded(4096)) }))
            .route("/exact", get(|| async { chunks(padded(MAX_FEED_BYTES)) }))
            .route("/big", get(|| async { chunks(padded(MAX_FEED_BYTES + 1)) }))
            .route(
                "/declared",
                get(|| async {
                    let pending =
                        futures::stream::pending::<Result<Bytes, std::convert::Infallible>>();
                    (
                        [(header::CONTENT_LENGTH, (MAX_FEED_BYTES + 1).to_string())],
                        Body::from_stream(pending),
                    )
                        .into_response()
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        (base, task)
    }

    #[tokio::test]
    async fn a_normal_feed_is_read_and_parsed() {
        let (base, server) = serve().await;
        let channel = fetch(&client().unwrap(), &format!("{base}/ok"))
            .await
            .unwrap();
        assert_eq!(channel.items().len(), 1);
        server.abort();
    }

    #[tokio::test]
    async fn a_feed_of_exactly_the_cap_is_still_read() {
        let (base, server) = serve().await;
        let channel = fetch(&client().unwrap(), &format!("{base}/exact"))
            .await
            .unwrap();
        assert_eq!(channel.items().len(), 1);
        server.abort();
    }

    #[tokio::test]
    async fn a_body_over_the_cap_is_refused_as_too_large() {
        let (base, server) = serve().await;
        let err = fetch(&client().unwrap(), &format!("{base}/big"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("larger than"),
            "unexpected failure: {err}"
        );
        assert!(!err.to_string().contains(&base));
        server.abort();
    }

    #[tokio::test]
    async fn a_declared_length_over_the_cap_is_refused_before_reading() {
        let (base, server) = serve().await;
        // The server never sends the body, so a client that reads it waits
        // for the whole request timeout.
        let err = tokio::time::timeout(
            Duration::from_secs(10),
            fetch(&client().unwrap(), &format!("{base}/declared")),
        )
        .await
        .expect("refused without reading the body")
        .unwrap_err();
        assert!(
            err.to_string().contains("larger than"),
            "unexpected failure: {err}"
        );
        server.abort();
    }

    #[tokio::test]
    async fn a_channel_redirected_to_another_host_is_read_without_its_url_going_along() {
        let hosts = testing::Redirect::start().await;
        let url = format!("{}/rss?passkey={}", hosts.base, testing::SECRET);
        let channel = fetch(&client().unwrap(), &url).await.unwrap();
        assert_eq!(channel.items().len(), 1);
        hosts.assert_nothing_leaked();
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
