//! The worlds the `되돌리기` tests start from: a third season whose first
//! releases were received under the app's offset, with their torrents seeding
//! in Transmission and their files on disk, and the helpers that read how an
//! undo ended.

use std::path::PathBuf;

use crate::{
    store::{
        channels::Rule,
        revisions::{NewRevision, RevisionState},
    },
    test_world::{show_hash, show_title, World},
};

/// One file of an undo: `(from, to, state)`.
pub(super) type UndoFile = (String, String, String);

pub(super) fn file(from: &str, to: &str, state: &str) -> UndoFile {
    (from.to_owned(), to.to_owned(), state.to_owned())
}

impl World {
    /// The season 3 folder of `Show`.
    pub fn season3(&self) -> PathBuf {
        self.media.join("Show/Season 03")
    }

    /// The names of the files in the season 3 folder, sorted.
    pub fn on_disk(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.season3())
            .map(|dir| {
                dir.map(|e| e.unwrap().file_name().into_string().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The content of a file of the season 3 folder.
    pub fn content(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.season3().join(name)).unwrap()
    }

    /// Every torrent has been received and is seeding.
    pub fn seeding(&self) {
        for t in self.tr.torrents() {
            self.tr.set_status(&t.hash, 6);
        }
    }

    /// The files of the undo the command `id` began: `(from, to, state)`.
    pub async fn undo_files(&self, id: &str) -> Vec<UndoFile> {
        self.ctx
            .channels
            .episode_undo(id)
            .await
            .unwrap()
            .expect("the undo began")
            .files
            .into_iter()
            .map(|f| (f.file.from_name, f.file.to_name, f.state.code().to_owned()))
            .collect()
    }

    /// Why the file of the undo `id` at `index` is as it is.
    pub async fn undo_reason(&self, id: &str, index: usize) -> String {
        self.ctx
            .channels
            .episode_undo(id)
            .await
            .unwrap()
            .expect("the undo began")
            .files[index]
            .reason
            .clone()
            .expect("a reason")
    }

    /// A video revision row of `episode_name` in the season 3 folder, for the
    /// item whose title has `part`.
    pub async fn revision_row(
        &self,
        rule: &Rule,
        part: &str,
        episode_name: &str,
        receiving: bool,
    ) -> i64 {
        let item = self.item_containing(part).await;
        self.ctx
            .revisions
            .create(
                self.now(),
                NewRevision {
                    item_id: item.id,
                    old_item_id: None,
                    rule_id: rule.id.clone(),
                    folder: self.season3().to_str().unwrap().to_owned(),
                    episode_name: episode_name.into(),
                    old_version: Some(1),
                    new_version: 2,
                    old_crc: None,
                    expected_crc: None,
                    torrent_hash: None,
                    state: if receiving {
                        RevisionState::Receiving
                    } else {
                        RevisionState::Skipped
                    },
                    reason: None,
                },
            )
            .await
            .unwrap()
            .id
    }

    /// The ids of the revision rows of `name` in the season 3 folder.
    pub async fn rows_of(&self, name: &str) -> Vec<i64> {
        self.ctx
            .revisions
            .of_episode(self.season3().to_str().unwrap().to_owned(), name.into())
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
    }

    /// Rewrites the identities the undo kept as a file system mounted again
    /// (after the machine restarted) shows the same files: another device
    /// number, the same inode, size and times.
    pub fn mounted_again(&self) {
        self.sql(
            "UPDATE episode_undo_files
                SET identity = '999999' || substr(identity, instr(identity, ':'))
              WHERE identity IS NOT NULL",
        );
    }

    /// Drops the extension of the RSS title `Show - {n}` was received for, as
    /// a feed without one has it.
    pub fn title_without_extension(&self, n: u32) {
        let title = show_title(n);
        let bare = title.trim_end_matches(".mkv").to_owned();
        let changed = rusqlite::Connection::open(self.db_path())
            .unwrap()
            .execute(
                "UPDATE history_items SET title = ?2 WHERE title = ?1",
                rusqlite::params![title, bare],
            )
            .unwrap();
        assert_eq!(changed, 1);
    }
}

/// Seasons 1 and 2 of 24 episodes, a third season's rule carried over with
/// `−24`, and its first releases `- 49` and `- 50` received under the app's
/// `−48`. The fake Transmission writes and renames the files.
pub(super) async fn third_season_received() -> (World, Rule) {
    let s = World::bare().await;
    for n in [49, 50, 51] {
        s.tr.content_on_add(&show_hash(n), format!("video {n}").as_bytes());
    }
    let place = s
        .library_of(&[(1, &["01", "02"]), (2, &["01", "02"])])
        .await;
    place.link(1, &[Some(24)]).await;
    place.link(2, &[Some(24)]).await;
    s.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, -24).await;
    s.feed_shows(&[]);
    s.cycle_later().await;
    s.feed_shows(&[49]);
    s.cycle_later().await;
    s.feed_shows(&[49, 50]);
    s.cycle_later().await;
    assert_eq!(s.torrent_names(), ["Show S03E01.mkv", "Show S03E02.mkv"]);
    assert_eq!(s.on_disk(), ["Show S03E01.mkv", "Show S03E02.mkv"]);
    s.seeding();
    (s, rule)
}

/// Seasons 1 and 2 of 18, a third season's rule carried over with `−24`,
/// `- 37` received as `S03E01` under the app's `−36` and `- 49` as `S03E13`,
/// which is the name `- 37` goes back to.
pub(super) async fn overlapping_names() -> (World, Rule) {
    let s = World::bare().await;
    for n in [37, 49] {
        s.tr.content_on_add(&show_hash(n), format!("video {n}").as_bytes());
    }
    let place = s
        .library_of(&[(1, &["01", "02"]), (2, &["01", "02"])])
        .await;
    place.link(1, &[Some(18)]).await;
    place.link(2, &[Some(18)]).await;
    s.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, -24).await;
    s.feed_shows(&[]);
    s.cycle_later().await;
    s.feed_shows(&[37]);
    s.cycle_later().await;
    s.feed_shows(&[37, 49]);
    s.cycle_later().await;
    assert_eq!(s.torrent_names(), ["Show S03E01.mkv", "Show S03E13.mkv"]);
    assert_eq!(s.stored_rule(&rule).await.episode, -36);
    s.seeding();
    (s, rule)
}
