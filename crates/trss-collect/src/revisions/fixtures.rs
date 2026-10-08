//! The release names, hashes and contents the revision tests share, and the
//! steps that bring a [`World`] to the states they start from. A name's
//! CRC32 is the CRC32 of the bytes its torrent writes, unless a test says
//! otherwise.

use crate::{
    store::{
        history::{HistoryResult, Observation},
        revisions::RevisionState,
    },
    test_world::{crc, magnet, World},
};

pub(super) const OLD_HASH: &str = "1111000000000000000000000000000000000014";
pub(super) const NEW_HASH: &str = "2222000000000000000000000000000000000014";
pub(super) const OTHER_HASH: &str = "3333000000000000000000000000000000000014";
pub(super) const V3_HASH: &str = "5555000000000000000000000000000000000014";
pub(super) const OLD_BYTES: &[u8] = b"episode 14, first release";
pub(super) const NEW_BYTES: &[u8] = b"episode 14, second release";
pub(super) const V3_BYTES: &[u8] = b"episode 14, third release";
pub(super) const EPISODE_NAME: &str = "Show S01E14.mkv";

pub(super) const ERAI_HASH: &str = "6666000000000000000000000000000000000006";
pub(super) const ERAI_EPISODE: &str = "Show S01E06.mkv";

/// `[SubsPlease] Show - 14<version> (1080p) [<crc>].mkv`.
pub(super) fn release(version: &str, crc: Option<&str>) -> String {
    match crc {
        Some(crc) => format!("[SubsPlease] Show - 14{version} (1080p) [{crc}].mkv"),
        None => format!("[SubsPlease] Show - 14{version} (1080p).mkv"),
    }
}

pub(super) fn v1() -> String {
    release("", Some(&crc(OLD_BYTES)))
}

pub(super) fn v2() -> String {
    release("v2", Some(&crc(NEW_BYTES)))
}

pub(super) fn v3() -> String {
    release("v3", Some(&crc(V3_BYTES)))
}

/// `[Erai-raws] Show - 06<version> [1080p CR WEBRip HEVC AAC][MultiSub][<crc>].mkv`:
/// `trname` alone reads `06v2` of this name as another episode, taken from
/// the CRC32 bracket.
pub(super) fn erai(version: &str) -> String {
    format!(
        "[Erai-raws] Show - 06{version} [1080p CR WEBRip HEVC AAC][MultiSub][{}].mkv",
        crc(NEW_BYTES)
    )
}

impl World {
    /// `14` received by a cycle and renamed to the episode name; its torrent
    /// stays in Transmission.
    pub async fn received_v1(&self) {
        self.feed(&[(OLD_HASH, &v1())]);
        self.tr.content_on_add(OLD_HASH, OLD_BYTES);
        self.cycle().await;
        self.complete(OLD_HASH);
        assert_eq!(self.names(), vec![EPISODE_NAME]);
        assert_eq!(self.tr.torrent(OLD_HASH).name, EPISODE_NAME);
    }

    /// The episode name holds `bytes`, a file of no torrent, and history knows
    /// the release `14` of the channel (seen in its feed earlier).
    pub async fn untracked_video(&self, bytes: &[u8]) {
        self.untracked_video_of(bytes, &v1()).await
    }

    /// [`World::untracked_video`] for the release named `title`.
    pub async fn untracked_video_of(&self, bytes: &[u8], title: &str) {
        std::fs::write(self.file(EPISODE_NAME), bytes).unwrap();
        self.ctx
            .history
            .record(
                1,
                vec![Observation {
                    channel_id: self.channel_id.clone(),
                    channel_label: "https://feeds.example.test/show".into(),
                    identity_key: "guid:v1".into(),
                    title: title.to_owned(),
                    link: magnet(OLD_HASH, title),
                    result: HistoryResult::NoMatch,
                    rule_id: None,
                    torrent_hash: None,
                    reason: None,
                }],
            )
            .await
            .unwrap();
    }

    /// The state of the replacement of the item titled `title`.
    pub async fn state_of(&self, title: &str) -> RevisionState {
        self.row_of(title).await.state
    }

    /// Whether the torrent `hash` was renamed onto the episode name.
    pub fn renamed_onto_episode(&self, hash: &str) -> bool {
        self.tr
            .calls_of("torrent-rename-path")
            .iter()
            .any(|c| c.args["ids"][0] == hash && c.args["name"] == EPISODE_NAME)
    }
}
