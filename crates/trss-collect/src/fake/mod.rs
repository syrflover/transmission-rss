//! Stand-ins for the servers the collection reads, for tests here and in the
//! crates above: [`FeedServer`] serves the RSS feeds of channels and
//! [`FakeNyaa`] the search RSS of a tracker, as the past search reads it. The
//! fake Transmission is in `trss_transmission::fake`. No test reaches a real
//! tracker.

mod feed;
mod nyaa;

pub use feed::FeedServer;
pub use nyaa::FakeNyaa;
