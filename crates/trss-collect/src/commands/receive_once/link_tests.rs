//! Getting the original link back for `다시 받기` (ticket 0008): the history
//! keeps a link with its secret values masked, so a retry fills them back from
//! the channel's URL when it can tell the result is the link the feed gave,
//! and reads the current feed when it cannot. A masked link is never handed
//! to Transmission. The feed is the fake feed server, and what each row
//! sets is what the link is like and what the feed does at the retry.

use trss_core::commands::CommandState;

use super::fixtures::*;
use crate::{
    store::history::HistoryResult,
    test_world::{feed_xml_of, World, SECRET},
};

/// A token that is the item's own and not the channel's.
const OWN: &str = "OWNTOKEN9876543210";

/// The host of the fake feed server, hence of the channel's URL.
const CHANNEL_HOST: &str = "127.0.0.1";

/// The reason a retry ends with when no link is to be had.
const NO_LINK: &str = "원래 링크를 되살리지 못했어요";

/// A `.torrent` download link on `host`, which the fake Transmission reads
/// like a magnet link.
fn download_link(host: &str, n: u32, name: &str, query: &str) -> String {
    let dn: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    format!("http://{host}/dl?xt=urn:btih:{}&dn={dn}{query}", hash(n))
}

/// What the feed does when the retry reads it.
#[derive(Debug, Clone, Copy)]
enum Feed {
    /// Serves what it served when the item was recorded.
    Same,
    /// Serves the other item only.
    WithoutTheItem,
    /// Answers 500.
    Down,
}

/// One kind of link and what happens to it.
struct Row {
    what: &'static str,
    /// The item's `<guid>`; empty for none, so that its identity is its link.
    guid: &'static str,
    host: &'static str,
    /// The part of the query that carries a secret.
    secret_query: String,
    feed: Feed,
    /// Whether the retry has to read the feed.
    reads_feed: bool,
    /// Whether the retry gets a link.
    recovered: bool,
}

fn rows() -> Vec<Row> {
    vec![
        Row {
            what: "a secret in a query of the channel's name is filled back from the channel",
            guid: "guid-liar-26",
            host: CHANNEL_HOST,
            secret_query: format!("&token={SECRET}"),
            feed: Feed::Same,
            reads_feed: false,
            recovered: true,
        },
        Row {
            what: "a link whose own value differs from the channel's is taken from the feed",
            guid: "",
            host: CHANNEL_HOST,
            secret_query: format!("&token={OWN}"),
            feed: Feed::Same,
            reads_feed: true,
            recovered: true,
        },
        Row {
            what: "a link on another host is not filled with the channel's secret",
            guid: "guid-liar-26",
            host: "elsewhere.test",
            secret_query: format!("&token={OWN}"),
            feed: Feed::Same,
            reads_feed: true,
            recovered: true,
        },
        Row {
            what: "a link on another host that left the feed is not received",
            guid: "guid-liar-26",
            host: "elsewhere.test",
            secret_query: format!("&token={OWN}"),
            feed: Feed::WithoutTheItem,
            reads_feed: true,
            recovered: false,
        },
        Row {
            what: "a secret under another name is found in the current feed",
            guid: "guid-liar-26",
            host: CHANNEL_HOST,
            secret_query: format!("&passkey={SECRET}"),
            feed: Feed::Same,
            reads_feed: true,
            recovered: true,
        },
        Row {
            what: "an item that left the feed cannot be recovered",
            guid: "guid-liar-26",
            host: CHANNEL_HOST,
            secret_query: format!("&passkey={SECRET}"),
            feed: Feed::WithoutTheItem,
            reads_feed: true,
            recovered: false,
        },
        Row {
            what: "a feed that cannot be read gives no link either",
            guid: "guid-liar-26",
            host: CHANNEL_HOST,
            secret_query: format!("&passkey={SECRET}"),
            feed: Feed::Down,
            reads_feed: true,
            recovered: false,
        },
    ]
}

#[tokio::test]
async fn a_retry_gets_the_original_link_back_only_from_where_it_can_be_trusted() {
    for row in rows() {
        let what = row.what;
        let link = download_link(row.host, 26, LIAR, &row.secret_query);
        let other_link = download_link(CHANNEL_HOST, 3, OTHER, "");
        let both = feed_xml_of(&[
            (row.guid, LIAR, &link),
            ("guid-other-3", OTHER, &other_link),
        ]);
        let s = World::with_rules(picked_rules()).await;
        s.feeds.set_xml("show", &both);
        s.tr.reject_adds(Some(crate::test_world::REFUSAL));
        s.cycle().await;
        s.tr.reject_adds(None);
        s.tr.clear_calls();
        let item = s.item_containing("LIAR GAME - 26").await;
        assert_eq!(item.result, HistoryResult::AddFailed, "{what}");
        assert!(item.link.contains("=***"), "{what}: {}", item.link);
        assert!(
            !item.link.contains(SECRET) && !item.link.contains(OWN),
            "{what}"
        );
        if row.guid.is_empty() {
            assert!(item.identity_key.starts_with("link:"), "{what}");
        }
        match row.feed {
            Feed::Same => {}
            Feed::WithoutTheItem => s.feeds.set_xml(
                "show",
                &feed_xml_of(&[("guid-other-3", OTHER, &other_link)]),
            ),
            Feed::Down => s.feeds.set_status("show", 500),
        }
        let feed_reads = s.feeds.hits("show");

        let finished = s.retry(item.id, CMD).await.unwrap();

        let adds = s.tr.calls_of("torrent-add");
        let held = s.item_containing("LIAR GAME - 26").await;
        assert_eq!(
            s.feeds.hits("show"),
            feed_reads + usize::from(row.reads_feed),
            "{what}: the feed is read only when it has to be"
        );
        if row.recovered {
            assert_eq!(adds.len(), 1, "{what}");
            assert_eq!(adds[0].args["filename"], link, "{what}");
            assert_eq!(finished.state, CommandState::Done, "{what}");
            assert_eq!(held.result, HistoryResult::Received, "{what}");
        } else {
            assert!(adds.is_empty(), "{what}: a masked link is never handed out");
            assert_eq!(finished.state, CommandState::Failed, "{what}");
            let reason = held.reason.as_deref().expect("a reason");
            assert!(reason.contains(NO_LINK), "{what}: {reason}");
            assert!(
                finished
                    .outcome
                    .reason
                    .as_deref()
                    .unwrap()
                    .contains(NO_LINK),
                "{what}"
            );
            assert_eq!(held.result, HistoryResult::AddFailed, "{what}");
        }
        // The channel's secret goes to no other host, and stays masked in
        // history afterwards.
        for call in s.tr.calls() {
            if row.host != CHANNEL_HOST {
                assert!(!call.args.to_string().contains(SECRET), "{what}: {call:?}");
            }
        }
        assert!(held.link.contains("=***"), "{what}");
        assert!(
            !format!("{:?}", s.history_items().await).contains(OWN),
            "{what}"
        );
        s.assert_secret_nowhere(CMD).await;
    }
}
