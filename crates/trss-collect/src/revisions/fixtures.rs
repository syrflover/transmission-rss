//! The release names, hashes and contents the revision tests share, and the
//! steps that bring a [`World`] to the states they start from. A name's
//! CRC32 is the CRC32 of the bytes its torrent writes, unless a test says
//! otherwise.

use std::path::{Path, PathBuf};

use crate::{
    commands::receive_once,
    store::{
        channels::RuleState,
        history::{HistoryResult, Observation},
        revisions::{Revision, RevisionState},
    },
    test_world::{crc, magnet, read, World},
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

pub(super) fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

/// The one replacement the to-do source lists as `받기 실패`.
pub(super) fn one_failure(failures: &[Revision]) -> &Revision {
    assert_eq!(failures.len(), 1, "{failures:?}");
    &failures[0]
}

impl World {
    /// `14` in place; `14v2` and `14v3` both received, `14v2` complete and
    /// skipped because `14v3` is still on its way.
    pub async fn v2_skipped_for_v3(&self) {
        self.received_v1().await;
        self.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
        self.tr.content_on_add(NEW_HASH, NEW_BYTES);
        self.tr.content_on_add(V3_HASH, V3_BYTES);
        self.tr.unfinished_on_add(NEW_HASH);
        self.tr.unfinished_on_add(V3_HASH);
        self.cycle().await;
        self.complete(NEW_HASH);
        self.cycle().await;
        assert_eq!(self.state_of(&v2()).await, RevisionState::Skipped);
        assert_eq!(self.state_of(&v3()).await, RevisionState::Receiving);
        assert_eq!(read(&self.file(EPISODE_NAME)), OLD_BYTES);
    }

    /// `14` in place; `14v2` removed `14`'s torrent but Transmission left its
    /// file, so `14v2` waits as removing.
    pub async fn removal_waits(&self) {
        self.received_v1().await;
        self.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
        self.tr.content_on_add(NEW_HASH, NEW_BYTES);
        self.cycle().await;
        self.complete(NEW_HASH);
        self.tr.keep_data_on_remove_of(OLD_HASH);
        self.cycle().await;
        self.cycle().await;
        assert_eq!(self.state_of(&v2()).await, RevisionState::Removing);
        assert!(!self.tr.torrents().iter().any(|t| t.hash == OLD_HASH));
        assert_eq!(read(&self.file(EPISODE_NAME)), OLD_BYTES);
    }

    /// How many torrent removals asked for the data to go too.
    pub fn removals_with_data(&self) -> usize {
        self.tr
            .calls_of("torrent-remove")
            .iter()
            .filter(|c| c.args["delete-local-data"] == true)
            .count()
    }

    /// `14v3` appears and is received and checked.
    pub async fn v3_received(&self) {
        self.feed(&[(V3_HASH, &v3()), (OLD_HASH, &v1())]);
        self.tr.content_on_add(V3_HASH, V3_BYTES);
        self.tr.unfinished_on_add(V3_HASH);
        self.cycle().await;
        assert_eq!(self.state_of(&v3()).await, RevisionState::Receiving);
        self.complete(V3_HASH);
    }

    /// `14v3` is received and checked while `14v2` waits for `14`'s file to
    /// go, so it waits as verified: one replacement of the episode at a time.
    /// Then `14`'s file goes and `14v2`'s rename is refused for now. Once
    /// `14v2` takes the name, `14v3` replaces it.
    pub async fn v3_verified_behind_v2(&self) {
        self.removal_waits().await;
        self.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
        self.tr.content_on_add(V3_HASH, V3_BYTES);
        self.tr.unfinished_on_add(V3_HASH);
        self.cycle().await;
        assert_eq!(self.state_of(&v3()).await, RevisionState::Receiving);
        self.complete(V3_HASH);
        self.cycle().await;
        assert_eq!(self.state_of(&v3()).await, RevisionState::Verified);

        self.tr.reject_rename_of(NEW_HASH, Some("busy"));
        std::fs::remove_file(self.file(EPISODE_NAME)).unwrap();
        self.cycle().await;
        assert_eq!(self.state_of(&v2()).await, RevisionState::Removed);
        assert_eq!(self.state_of(&v3()).await, RevisionState::Verified);
    }

    /// `14v2` removed `14` and its rename is refused for now.
    pub async fn v2_waits_for_its_name(&self) {
        self.received_v1().await;
        self.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
        self.tr.content_on_add(NEW_HASH, NEW_BYTES);
        self.cycle().await;
        self.complete(NEW_HASH);
        self.tr.reject_rename_of(NEW_HASH, Some("busy"));
        self.cycle().await;
        assert_eq!(self.state_of(&v2()).await, RevisionState::Removed);
        assert_eq!(self.names(), vec![v2()]);
    }

    /// Where a test puts `14v2`'s file while it is "missing".
    pub fn away(&self) -> PathBuf {
        self.season.parent().unwrap().join("away.mkv")
    }

    /// Takes the season folder away (a mount that is not there) and returns
    /// where it went.
    pub fn folder_away(&self) -> PathBuf {
        let elsewhere = self.season.with_file_name("Season 01 away");
        std::fs::rename(&self.season, &elsewhere).unwrap();
        elsewhere
    }

    pub fn folder_back(&self, elsewhere: &Path) {
        std::fs::rename(elsewhere, &self.season).unwrap();
    }

    /// Archives the rule of `14` (`보관`): the rule is off and its work
    /// folder is in the archive folder. Returns where the folder went.
    pub async fn archive(&self) -> PathBuf {
        let rule_id = self.row_of(&v2()).await.rule_id;
        self.ctx
            .channels
            .set_rule_state(&rule_id, RuleState::Archived, self.now())
            .await
            .unwrap();
        let work = self.season.parent().unwrap();
        let archive = self.dir.path().join("archive");
        std::fs::create_dir_all(&archive).unwrap();
        let archived = archive.join("Show");
        std::fs::rename(work, &archived).unwrap();
        archived
    }

    /// Restores the rule archived with [`World::archive`] (`복원`).
    pub async fn restore(&self, archived: &Path) {
        std::fs::rename(archived, self.season.parent().unwrap()).unwrap();
        let rule_id = self.row_of(&v2()).await.rule_id;
        self.ctx
            .channels
            .set_rule_state(&rule_id, RuleState::Active, self.now())
            .await
            .unwrap();
    }

    /// Whether `다시 받기` is offered for the item titled `title`: a plan
    /// exists for it, by its row and its rule.
    pub async fn can_retry(&self, title: &str) -> bool {
        let item = self.item(title).await;
        let channel = self
            .ctx
            .channels
            .get_channel(&item.channel_id)
            .await
            .unwrap();
        let rule = match &item.rule_id {
            Some(id) => self.ctx.channels.get_rule(id).await.unwrap(),
            None => None,
        };
        let revision = receive_once::revision_retry(&self.ctx.revisions, item.id)
            .await
            .unwrap();
        receive_once::retry_plan_for(&item, channel.as_ref(), rule.as_ref(), &revision).is_ok()
    }
}
