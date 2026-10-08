//! Erai-raws' magnet feed through the revision decision ([`plan`]): its item
//! titles have no extension and no CRC32, write a revision as `- 01 (V2)` and
//! end in a list of subtitle languages that differs between an episode and its
//! revision (ticket 0115; `docs/specs/collection.md`, 영상 수정본의 대체). The
//! names are the pair of the release-name corpus.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use trss_core::{folder_locks::FolderLocks, Db};
use trss_transmission::{
    fake::{FakeTorrent, FakeTransmission},
    Redactor,
};

use super::*;
use crate::{
    context::TransmissionLink,
    store::{
        channels::ChannelStore,
        history::{HistoryResult, HistoryStore, Observation},
        revisions::{Replacement, Revision, RevisionState, RevisionStore, RowWrite},
    },
};

const FIRST: &str = "[Magnet] Kusuriya no Hitorigoto 3rd Season - 01 [1080p CR WEB-DL AVC AAC][us][br][mx][es][sa][fr][de][it][ru][Airing]";
const SECOND: &str = "[Magnet] Kusuriya no Hitorigoto 3rd Season - 01 (V2) [1080p CR WEB-DL AVC AAC][us][br][mx][es][sa][fr][de][it][ru][pl][Airing]";
const HASH: &str = "aaaa000000000000000000000000000000000001";
const NEW_HASH: &str = "bbbb000000000000000000000000000000000002";
/// The files an Erai-raws magnet torrent holds, as it names them.
const NEW_MKV: &str =
    "[Erai-raws] Kusuriya no Hitorigoto 3rd Season - 01v2 [1080p CR WEBRip HEVC AAC][MultiSub].mkv";
const NEW_MP4: &str =
    "[Erai-raws] Kusuriya no Hitorigoto 3rd Season - 01v2 [1080p CR WEBRip HEVC AAC][MultiSub].mp4";
const EPISODE_MKV: &str = "Kusuriya no Hitorigoto 3rd Season S01E01.mkv";
const CHANNEL: &str = "erai";

/// A fake Transmission, an app database in memory and the rule folder
/// `.../Kusuriya no Hitorigoto 3rd Season/Season 01`.
struct World {
    tr: FakeTransmission,
    ctx: RevisionsContext,
    season: PathBuf,
    _root: TempDir,
}

impl World {
    async fn new() -> World {
        let tr = FakeTransmission::start().await;
        let db = Db::open_blocking(":memory:").unwrap();
        let root = tempfile::tempdir().unwrap();
        let season = root
            .path()
            .join("Kusuriya no Hitorigoto 3rd Season/Season 01");
        std::fs::create_dir_all(&season).unwrap();
        let ctx = RevisionsContext {
            channels: ChannelStore::new(db.clone()),
            history: HistoryStore::new(db.clone()),
            revisions: RevisionStore::new(db),
            transmission: TransmissionLink {
                url: tr.url().parse().unwrap(),
                http: trss_transmission::http_client(Duration::from_secs(5)).unwrap(),
            },
        };
        tr.on_disk(root.path());
        World {
            tr,
            ctx,
            season,
            _root: root,
        }
    }

    /// The first release of the episode was received as `title` and named by
    /// the rule's cycle: the video is `file` in the folder, the single file of
    /// the torrent `HASH` that history records for the item.
    async fn received(&self, title: &str, file: &str) {
        std::fs::write(self.season.join(file), b"episode 1").unwrap();
        self.tr
            .preload(FakeTorrent::new(HASH, file).in_dir(&self.season).bot());
        let observation = Observation {
            channel_id: CHANNEL.to_owned(),
            channel_label: "https://feed.test/erai".to_owned(),
            identity_key: "guid-first".to_owned(),
            title: title.to_owned(),
            link: format!("magnet:?xt=urn:btih:{HASH}"),
            result: HistoryResult::Received,
            rule_id: Some("rule".to_owned()),
            torrent_hash: Some(HASH.to_owned()),
            reason: None,
        };
        self.ctx.history.record(1, vec![observation]).await.unwrap();
    }

    /// The `(V2)` item was held back as `버전 미상` by a cycle and the person
    /// asks for `다시 받기`: its torrent `NEW_HASH` holds the single file
    /// `new_file` in the folder, and the row is confirmed with it.
    async fn confirmed(&self, new_file: &str) -> i64 {
        let observation = Observation {
            channel_id: CHANNEL.to_owned(),
            channel_label: "https://feed.test/erai".to_owned(),
            identity_key: "guid-second".to_owned(),
            title: SECOND.to_owned(),
            link: format!("magnet:?xt=urn:btih:{NEW_HASH}"),
            result: HistoryResult::VersionUnknown,
            rule_id: Some("rule".to_owned()),
            torrent_hash: None,
            reason: Some(NO_CRC.to_owned()),
        };
        let (item_id, _) = self.ctx.history.record_one(2, observation).await.unwrap();
        let Plan::Unknown(decided, reason) = self.plan(SECOND).await else {
            panic!("not version unknown")
        };
        let row = decided.row(
            item_id,
            "rule".to_owned(),
            &self.season,
            RevisionState::Unknown,
            Some(reason.to_owned()),
            None,
        );
        self.ctx.revisions.create(2, row).await.unwrap();

        std::fs::write(self.season.join(new_file), b"episode 1, second release").unwrap();
        self.tr.preload(
            FakeTorrent::new(NEW_HASH, new_file)
                .in_dir(&self.season)
                .bot()
                .status(6),
        );
        self.ctx
            .revisions
            .confirm(item_id, 3, NEW_HASH.to_owned(), confirm_crc())
            .await
            .unwrap()
            .expect("the row is confirmed");
        item_id
    }

    /// The worker's replacement steps, as often as a cycle runs them.
    async fn advance(&self) {
        let locks = FolderLocks::new();
        for at in 10..14 {
            advance(
                &self.ctx,
                &locks,
                at,
                &Redactor::none(),
                &CancellationToken::new(),
            )
            .await;
        }
    }

    async fn row(&self, item_id: i64) -> Revision {
        self.ctx.revisions.by_item(item_id).await.unwrap().unwrap()
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.season)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// What the cycle decides about the feed item `title`.
    async fn plan(&self, title: &str) -> Plan {
        let selected = Selected {
            channel_id: CHANNEL,
            identity_key: "guid-second",
            title,
            save_path: &self.season,
            episode: 0,
        };
        plan(&self.ctx, &selected, &Listing::new()).await
    }
}

#[tokio::test]
async fn a_magnet_revision_of_a_received_episode_is_not_received_and_is_version_unknown() {
    let w = World::new().await;
    w.received(FIRST, "Kusuriya no Hitorigoto 3rd Season S01E01.mkv")
        .await;

    // The second list of languages has `[pl]`, which the first has not.
    let Plan::Unknown(decided, reason) = w.plan(SECOND).await else {
        panic!("not version unknown")
    };
    assert_eq!(reason, NO_CRC);
    assert_eq!(
        decided.episode_name,
        "Kusuriya no Hitorigoto 3rd Season S01E01.mkv"
    );
    assert_eq!((decided.version, decided.crc), (2, None));
    assert_eq!(decided.old_version, Some(1));
    assert!(decided.old_item_id.is_some());
}

#[tokio::test]
async fn a_magnet_revision_of_an_episode_not_received_yet_is_an_ordinary_item() {
    let w = World::new().await;
    assert!(is_revision(SECOND));
    assert_eq!(w.plan(SECOND).await, Plan::Normal);

    // Another episode of the work in the folder does not make it a revision.
    let other = "Kusuriya no Hitorigoto 3rd Season S01E02.mkv";
    w.received(FIRST, other).await;
    assert_eq!(w.plan(SECOND).await, Plan::Normal);
}

#[tokio::test]
async fn a_magnet_revision_finds_the_video_whatever_its_extension() {
    let w = World::new().await;
    let file = "Kusuriya no Hitorigoto 3rd Season S01E01.mp4";
    w.received(FIRST, file).await;
    let Plan::Unknown(decided, reason) = w.plan(SECOND).await else {
        panic!("not version unknown")
    };
    assert_eq!((decided.episode_name.as_str(), reason), (file, NO_CRC));
}

#[tokio::test]
async fn a_magnet_revision_of_an_episode_with_two_videos_is_version_unknown() {
    let w = World::new().await;
    let base = "Kusuriya no Hitorigoto 3rd Season S01E01";
    w.received(FIRST, &format!("{base}.mkv")).await;
    std::fs::write(w.season.join(format!("{base}.mp4")), b"another").unwrap();

    let Plan::Unknown(decided, reason) = w.plan(SECOND).await else {
        panic!("not version unknown")
    };
    assert_eq!(reason, SEVERAL_VIDEOS);
    assert_eq!(decided.episode_name, format!("{base}.mkv"));
    assert_eq!(decided.version, 2);
}

#[tokio::test]
async fn another_groups_magnet_revision_of_the_episode_is_not_this_ones_revision() {
    let w = World::new().await;
    w.received(FIRST, "Kusuriya no Hitorigoto 3rd Season S01E01.mkv")
        .await;
    // The same episode from another group: a duplicate, received as before.
    let other = SECOND.replacen("[Magnet]", "[Other]", 1);
    assert_eq!(w.plan(&other).await, Plan::Normal);
}

/// The file an episode title without an extension names is found by the name
/// `trname` gives it, with a video extension.
#[test]
fn a_title_without_an_extension_names_the_one_video_file_of_its_episode_name() {
    let root = tempfile::tempdir().unwrap();
    let season = root.path().join("Show/Season 01");
    let title = "[Magnet] Show - 01 (V2) [1080p CR WEB-DL AVC AAC][us][pl][Airing]";
    let named = |season: &Path| episode_file(season, title, 0).unwrap();

    // No folder yet, an empty one, and one with other episodes and files that
    // only look like it.
    assert_eq!(named(&season), EpisodeFile::Absent);
    std::fs::create_dir_all(season.join("Show S01E01.mp4")).unwrap();
    for other in [
        "Show S01E02.mkv",
        "Show S01E010.mkv",
        "Show S01E01 extra.mkv",
        "Show S01E01.srt",
        "Show S01E01",
        "Show S01E01.mkv.part",
    ] {
        std::fs::write(season.join(other), b"x").unwrap();
    }
    assert_eq!(named(&season), EpisodeFile::Absent, "a folder is no video");

    std::fs::write(season.join("Show S01E01.mkv"), b"x").unwrap();
    assert_eq!(named(&season), EpisodeFile::One("Show S01E01.mkv".into()));
    for name in ["Show S01E01.mkv", "Show S01E01.MKV"] {
        assert!(names_file(&season, title, 0, name), "{name}");
    }
    for name in ["Show S01E02.mkv", "Show S01E01.srt", "Show S01E010.mkv"] {
        assert!(!names_file(&season, title, 0, name), "{name}");
    }

    // The rule's conversion applies to the episode of the name.
    assert_eq!(
        episode_file(&season, title, 1).unwrap(),
        EpisodeFile::One("Show S01E01.mkv".into())
    );
    // Episode 1 with a conversion that starts at 2 is episode 2.
    assert_eq!(
        episode_file(&season, title, 2).unwrap(),
        EpisodeFile::One("Show S01E02.mkv".into())
    );

    std::fs::remove_dir(season.join("Show S01E01.mp4")).unwrap();
    std::fs::write(season.join("Show S01E01.MP4"), b"x").unwrap();
    assert_eq!(
        named(&season),
        EpisodeFile::Several(vec!["Show S01E01.MP4".into(), "Show S01E01.mkv".into()])
    );

    // A folder `trname` has no name for gives none.
    assert_eq!(
        episode_file(root.path(), title, 0).unwrap(),
        EpisodeFile::Unnamed
    );
}

/// A title with an extension names its file as it always has, whether or not
/// the folder has it.
#[test]
fn a_title_with_an_extension_names_its_file_as_before() {
    let season = Path::new("/media/Show/Season 01");
    let title = "[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv";
    assert_eq!(
        episode_file(season, title, 0).unwrap(),
        EpisodeFile::One("Show S01E14.mkv".into())
    );
    assert!(names_file(season, title, 0, "Show S01E14.mkv"));
    assert!(!names_file(season, title, 0, "Show S01E14.mp4"));
    assert_eq!(
        episode_file(Path::new("/media"), title, 0).unwrap(),
        EpisodeFile::Unnamed
    );
}

/// What `다시 받기` writes for an item whose name has no CRC32.
fn confirm_crc() -> Option<String> {
    match confirm(SECOND, NEW_HASH) {
        RowWrite::Confirm { expected_crc, .. } => expected_crc,
        other => panic!("{other:?}"),
    }
}

/// `다시 받기` of the Erai `버전 미상` item replaces the video: the
/// release names carry no extension and different language lists, and the
/// old video is found as the same release through them.
#[tokio::test]
async fn a_confirmed_magnet_revision_replaces_the_video_of_the_same_release() {
    let w = World::new().await;
    w.received(FIRST, EPISODE_MKV).await;
    let item_id = w.confirmed(NEW_MKV).await;
    assert_eq!(confirm_crc(), None);

    w.advance().await;

    let row = w.row(item_id).await;
    assert_eq!(row.state, RevisionState::Done, "{row:?}");
    assert_eq!(w.names(), [EPISODE_MKV]);
    assert_eq!(
        std::fs::read(w.season.join(EPISODE_MKV)).unwrap(),
        b"episode 1, second release"
    );
    let removed = w.tr.calls_of("torrent-remove");
    assert_eq!(removed.len(), 1, "the old torrent is removed: {removed:?}");
}

/// The new video takes the episode's name, so a file of another extension is
/// not put under it: both files stay and the replacement fails.
#[tokio::test]
async fn a_confirmed_magnet_revision_of_another_extension_replaces_nothing() {
    let w = World::new().await;
    w.received(FIRST, EPISODE_MKV).await;
    let item_id = w.confirmed(NEW_MP4).await;

    w.advance().await;

    let row = w.row(item_id).await;
    assert_eq!(row.state, RevisionState::Failed, "{row:?}");
    assert_eq!(
        row.reason.as_deref(),
        Some("받은 새 영상의 확장자(.mp4)가 회차 이름의 영상(.mkv)과 달라서 대체하지 않았어요. 이전 영상은 그대로 있어요.")
    );
    // Like a CRC32 that differs: the new video was received, so it is no
    // failure `다시 받기` receives again.
    assert_eq!(row.received_name.as_deref(), Some(NEW_MP4));
    assert!(row.is_failure() && !row.not_received());
    assert!(!received_again_on_retry(&row));
    let mut files = vec![EPISODE_MKV, NEW_MP4];
    files.sort();
    assert_eq!(w.names(), files, "both files stay");
    assert!(w.tr.calls_of("torrent-remove").is_empty());
    assert!(w.tr.calls_of("torrent-rename-path").is_empty());
    assert!(w.tr.torrents().iter().any(|t| t.hash == HASH));
}

/// The extension is compared without regard to case.
#[tokio::test]
async fn the_extension_of_the_new_video_is_compared_without_regard_to_case() {
    let w = World::new().await;
    let episode = "Kusuriya no Hitorigoto 3rd Season S01E01.MKV";
    w.received(FIRST, episode).await;
    let item_id = w.confirmed(NEW_MKV).await;
    w.advance().await;
    let row = w.row(item_id).await;
    assert_eq!(row.state, RevisionState::Done, "{row:?}");
    assert_eq!(w.names(), [episode]);
}

/// A lower revision of a release that replaced a video is not received into
/// the folder again, whatever language list its title carries.
#[test]
fn a_lower_revision_with_other_language_tags_is_held_back_by_the_release_that_replaced() {
    let folder = Path::new("/media/Kusuriya/Season 01");
    let replaced = Replaced::new(vec![Replacement {
        folder: folder.to_string_lossy().into_owned(),
        title: SECOND.to_owned(),
        new_version: 2,
    }]);
    // The episode's first release, without the `[pl]` of the revision.
    assert!(replaced.holds_higher(folder, FIRST));
    // The revision itself, another folder, and another group's release.
    assert!(!replaced.holds_higher(folder, SECOND));
    assert!(!replaced.holds_higher(Path::new("/media/Other/Season 01"), FIRST));
    assert!(!replaced.holds_higher(folder, &FIRST.replacen("[Magnet]", "[Other]", 1)));
}
