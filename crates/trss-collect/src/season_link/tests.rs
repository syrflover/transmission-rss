//! Connecting a subscription to the season its videos appeared in, in the
//! world of the collect tests: a real library scan of temporary folders, the
//! history of what a rule received, and a fake Transmission that says where
//! each torrent's files are. The worker's tests keep when the pass runs in a
//! cycle and what it starts after a connection (the subtitle lines, the
//! creator's jobs).
//!
//! The season comes only from the videos a rule itself received: a folder of
//! the same name that holds a person's videos, or videos other rules received,
//! connect nothing.

use std::path::{Path, PathBuf};

use trss_library::discovery;
use trss_transmission::fake::FakeTorrent;

use super::*;
use crate::{
    store::{
        channels::Rule,
        history::{HistoryResult, Observation},
    },
    test_world::{write, World},
};

struct Scene {
    world: World,
    /// The watch folder, which holds the work folders the torrents save into.
    shows: PathBuf,
    next_hash: u32,
}

impl Scene {
    async fn new() -> Scene {
        let world = World::bare().await;
        let shows = world.media.clone();
        Scene {
            world,
            shows,
            next_hash: 0,
        }
    }

    /// A subscription rule for anime `no`.
    async fn subscription(&self, no: i64, phrase: &str) -> Rule {
        self.world
            .subscribe(phrase, &format!("{phrase}/Season 01"), no, 0)
            .await
    }

    /// The rule received a torrent whose file is `file` (a path below the
    /// watch folder); Transmission has it saved next to the file.
    async fn received(&mut self, rule: &Rule, file: &str) {
        self.next_hash += 1;
        let hash = format!("{:040x}", self.next_hash);
        let path = Path::new(file);
        let dir = self.shows.join(path.parent().unwrap());
        write(&self.shows.join(file), "x");
        self.world.tr.preload(
            FakeTorrent::new(&hash, "name").in_dir(&dir).files(&[path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()]),
        );
        self.world
            .ctx
            .history
            .record(
                self.world.now(),
                vec![Observation {
                    channel_id: self.world.channel_id.clone(),
                    channel_label: "feed".into(),
                    identity_key: format!("key-{hash}"),
                    title: format!("[Group] {file}"),
                    link: format!("magnet:?xt=urn:btih:{hash}"),
                    result: HistoryResult::Received,
                    rule_id: Some(rule.id.clone()),
                    torrent_hash: Some(hash),
                    reason: None,
                }],
            )
            .await
            .unwrap();
    }

    /// Registers the watch folder (which reads it) once the files are there.
    async fn register(&self) {
        let scan = discovery::scan(&self.shows).unwrap();
        self.world
            .ctx
            .library
            .add_folder(self.shows.to_str().unwrap().to_owned(), scan, 100, &[])
            .await
            .unwrap();
    }

    /// The worker's reading of the watch folder, after the files changed.
    async fn rescan(&self) {
        let library = &self.world.ctx.library;
        let folder = library.folders().await.unwrap().remove(0);
        let scan = discovery::scan(&self.shows).unwrap();
        library
            .record_scan(&folder.id, Ok(scan), self.world.now())
            .await
            .unwrap();
    }

    async fn link(&self) -> Linked {
        link_seasons(&self.world.ctx.link()).await
    }

    async fn work_id(&self, name: &str) -> String {
        let library = &self.world.ctx.library;
        let folder = library.folders().await.unwrap().remove(0);
        library
            .works(&folder.id)
            .await
            .unwrap()
            .into_iter()
            .find(|w| w.dir_name == name)
            .unwrap_or_else(|| panic!("no work {name}"))
            .id
    }

    async fn rule(&self, rule: &Rule) -> Rule {
        self.world.stored_rule(rule).await
    }

    /// How many times the season link asked Transmission for torrents with
    /// their files.
    fn file_reads(&self) -> usize {
        self.world
            .tr
            .calls_of("torrent-get")
            .iter()
            .filter(|call| {
                call.args["fields"]
                    .as_array()
                    .is_some_and(|fields| fields.iter().any(|f| f == "files"))
            })
            .count()
    }

    /// The torrent the last `received` added is saved in `dir` for
    /// Transmission, not where the library has the file.
    fn saved_elsewhere(&self, dir: &Path, file: &str) {
        let hash = format!("{:040x}", self.next_hash);
        self.world.tr.remove(&hash);
        self.world
            .tr
            .preload(FakeTorrent::new(&hash, "name").in_dir(dir).files(&[file]));
    }
}

fn season_of(rule: &Rule) -> Option<&str> {
    rule.subscription.as_ref().unwrap().season_id.as_deref()
}

fn blocked_of(rule: &Rule) -> Option<&str> {
    rule.subscription
        .as_ref()
        .unwrap()
        .season_blocked
        .as_deref()
}

#[tokio::test]
async fn a_subscription_is_connected_to_the_season_folder_its_received_video_appeared_in() {
    let mut scene = Scene::new().await;
    let rule = scene.subscription(7, "Work").await;
    scene
        .received(&rule, "Work/Season 01/Work - S01E01.mkv")
        .await;
    scene.register().await;

    let linked = scene.link().await;

    assert_eq!((linked.linked, linked.taken), (1, 0));
    assert_eq!(linked.anime_nos, [7]);
    let work = scene.work_id("Work").await;
    let connected = scene.rule(&rule).await;
    assert_eq!(season_of(&connected), Some(format!("{work}:1").as_str()));
    assert_eq!(blocked_of(&connected), None);

    // Connected once, it stays through the passes that follow, whatever the
    // folders do.
    scene
        .received(&rule, "Work/Season 02/Work - S02E01.mkv")
        .await;
    std::fs::remove_dir_all(scene.shows.join("Work/Season 01")).unwrap();
    scene.rescan().await;
    scene.link().await;
    scene.link().await;
    let still = scene.rule(&rule).await;
    assert_eq!(season_of(&still), Some(format!("{work}:1").as_str()));
}

#[tokio::test]
async fn videos_a_person_put_in_or_other_rules_received_connect_nothing() {
    let mut scene = Scene::new().await;
    // Both rules save into a folder named like the work, as a person would.
    let waiting = scene.subscription(7, "Work").await;
    let other = scene.subscription(8, "Other").await;
    write(&scene.shows.join("Work/Season 01/Work - S01E01.mkv"), "x");
    scene
        .received(&other, "Work/Season 01/Work - S01E02.mkv")
        .await;
    // A video a rule received that no folder of the library holds (yet).
    let lost = scene.subscription(9, "Lost").await;
    scene
        .received(&lost, "Lost/Season 01/Lost - S01E01.mkv")
        .await;
    std::fs::remove_file(scene.shows.join("Lost/Season 01/Lost - S01E01.mkv")).unwrap();
    scene.register().await;

    scene.link().await;

    assert_eq!(
        season_of(&scene.rule(&waiting).await),
        None,
        "a person's video in a folder of the rule's name"
    );
    assert_eq!(
        season_of(&scene.rule(&lost).await),
        None,
        "a video the library does not hold"
    );
    // The rule that did receive the video is the one connected.
    let work = scene.work_id("Work").await;
    assert_eq!(
        season_of(&scene.rule(&other).await),
        Some(format!("{work}:1").as_str())
    );
}

#[tokio::test]
async fn a_season_another_anime_holds_is_not_taken_and_the_rule_notes_it() {
    let mut scene = Scene::new().await;
    let first = scene.subscription(7, "First").await;
    let second = scene.subscription(8, "Second").await;
    scene
        .received(&first, "Work/Season 01/Work - S01E01.mkv")
        .await;
    scene
        .received(&second, "Work/Season 01/Work - S01E02.mkv")
        .await;
    scene.register().await;

    let linked = scene.link().await;

    let work = scene.work_id("Work").await;
    let season = format!("{work}:1");
    let (first, second) = (scene.rule(&first).await, scene.rule(&second).await);
    // Rules are connected in their order: the first takes the season.
    assert_eq!((linked.linked, linked.taken), (1, 1));
    assert_eq!(season_of(&first), Some(season.as_str()));
    assert_eq!(season_of(&second), None);
    assert_eq!(blocked_of(&second), Some(season.as_str()));

    // The anime that holds the season is known for the rule's detail.
    assert_eq!(
        scene
            .world
            .ctx
            .channels
            .season_holder(&season)
            .await
            .unwrap(),
        Some(7)
    );
}

#[tokio::test]
async fn videos_in_more_than_one_season_leave_the_rule_unconnected() {
    let mut scene = Scene::new().await;
    let rule = scene.subscription(7, "Work").await;
    scene
        .received(&rule, "Work/Season 01/Work - S01E12.mkv")
        .await;
    scene
        .received(&rule, "Work/Season 02/Work - S02E01.mkv")
        .await;
    scene.register().await;

    let linked = scene.link().await;

    assert_eq!((linked.linked, linked.taken), (0, 0));
    assert_eq!(season_of(&scene.rule(&rule).await), None);
}

#[tokio::test]
async fn a_paused_rule_that_received_videos_is_connected_too() {
    let mut scene = Scene::new().await;
    let rule = scene.subscription(7, "Work").await;
    scene
        .received(&rule, "Work/Season 01/Work - S01E01.mkv")
        .await;
    scene
        .world
        .ctx
        .channels
        .set_video_receiving(&rule.id, rule.version, false, 0)
        .await
        .unwrap();
    scene.register().await;

    scene.link().await;

    let work = scene.work_id("Work").await;
    assert_eq!(
        season_of(&scene.rule(&rule).await),
        Some(format!("{work}:1").as_str())
    );
}

// --- what the pass does not repeat ----------------------------------------------

#[tokio::test]
async fn a_rule_that_cannot_be_connected_costs_nothing_until_something_it_depends_on_changes() {
    let mut scene = Scene::new().await;
    let lost = scene.subscription(9, "Lost").await;
    let first = "Lost/Season 01/Lost - S01E01.mkv";
    scene.received(&lost, first).await;
    // The video is not in the library: not there when the folder is read.
    std::fs::remove_file(scene.shows.join(first)).unwrap();
    scene.register().await;

    scene.link().await;
    assert_eq!(scene.file_reads(), 1, "the first attempt asks Transmission");
    scene.link().await;
    scene.link().await;
    assert_eq!(
        scene.file_reads(),
        1,
        "nothing it depends on changed, so there is nothing to ask"
    );

    // Another video it received: its torrents are a different set.
    scene
        .received(&lost, "Lost/Season 01/Lost - S01E02.mkv")
        .await;
    std::fs::remove_file(scene.shows.join("Lost/Season 01/Lost - S01E02.mkv")).unwrap();
    scene.link().await;
    assert_eq!(scene.file_reads(), 2, "a new received item");
    scene.link().await;
    assert_eq!(scene.file_reads(), 2);

    // The library changes: the first video turns up.
    write(&scene.shows.join(first), "x");
    scene.rescan().await;
    scene.link().await;
    assert_eq!(scene.file_reads(), 3, "the library changed");
    let work = scene.work_id("Lost").await;
    assert_eq!(
        season_of(&scene.rule(&lost).await),
        Some(format!("{work}:1").as_str())
    );
    // Connected, it is out of the pass.
    scene.link().await;
    assert_eq!(scene.file_reads(), 3);
}

#[tokio::test]
async fn a_rule_with_videos_in_several_seasons_and_a_rule_whose_season_is_taken_are_not_retried() {
    let mut scene = Scene::new().await;
    let several = scene.subscription(7, "Several").await;
    scene
        .received(&several, "Several/Season 01/Several - S01E12.mkv")
        .await;
    scene
        .received(&several, "Several/Season 02/Several - S02E01.mkv")
        .await;
    let holder = scene.subscription(8, "Holder").await;
    let taken = scene.subscription(9, "Taken").await;
    scene
        .received(&holder, "Same/Season 01/Same - S01E01.mkv")
        .await;
    scene
        .received(&taken, "Same/Season 01/Same - S01E02.mkv")
        .await;
    scene.register().await;

    scene.link().await;
    assert_eq!(scene.file_reads(), 1);
    let work = scene.work_id("Same").await;
    assert_eq!(
        season_of(&scene.rule(&holder).await),
        Some(format!("{work}:1").as_str())
    );
    let blocked = scene.rule(&taken).await;
    assert_eq!(blocked_of(&blocked), Some(format!("{work}:1").as_str()));

    scene.link().await;
    scene.link().await;

    assert_eq!(scene.file_reads(), 1);
    assert_eq!(season_of(&scene.rule(&several).await), None);
    // The note on the taken rule is not written again either.
    assert_eq!(scene.rule(&taken).await.version, blocked.version);
}

// --- a path Transmission and the library do not agree on -------------------------

#[tokio::test]
async fn a_rule_whose_torrents_are_found_nowhere_in_the_library_is_reported_once() {
    let mut scene = Scene::new().await;
    let mapped = scene.subscription(7, "Mapped").await;
    let fine = scene.subscription(8, "Fine").await;
    scene
        .received(&mapped, "Mapped/Season 01/Mapped - S01E01.mkv")
        .await;
    // Transmission keeps its data where this app sees other folders.
    let elsewhere = scene.world.dir.path().join("downloads/Mapped/Season 01");
    scene.saved_elsewhere(&elsewhere, "Mapped - S01E01.mkv");
    scene
        .received(&fine, "Fine/Season 01/Fine - S01E01.mkv")
        .await;
    scene.register().await;

    let first = scene.link().await;
    assert_eq!(first.unmatched, std::slice::from_ref(&mapped.id));
    assert_eq!((first.linked, first.taken), (1, 0));
    assert_eq!(season_of(&scene.rule(&mapped).await), None);

    // It is tried again when it receives something new, and says nothing again.
    scene
        .received(&mapped, "Mapped/Season 01/Mapped - S01E02.mkv")
        .await;
    scene.saved_elsewhere(&elsewhere, "Mapped - S01E02.mkv");
    let again = scene.link().await;
    assert_eq!(scene.file_reads(), 2, "the new item makes it a candidate");
    assert!(again.unmatched.is_empty(), "{again:?}");
}

// --- a note on a season that is not held any more ---------------------------------

#[tokio::test]
async fn the_note_of_a_taken_season_goes_when_nothing_holds_the_season_any_more() {
    let mut scene = Scene::new().await;
    let first = scene.subscription(7, "First").await;
    let second = scene.subscription(8, "Second").await;
    scene
        .received(&first, "Work/Season 01/Work - S01E01.mkv")
        .await;
    scene
        .received(&second, "Work/Season 01/Work - S01E02.mkv")
        .await;
    scene.register().await;
    scene.link().await;
    let work = scene.work_id("Work").await;
    let noted = scene.rule(&second).await;
    assert_eq!(blocked_of(&noted), Some(format!("{work}:1").as_str()));

    // The holder goes, and the second rule's torrent is gone from Transmission
    // too, so the rule cannot take the season itself. The season keeps the
    // holder's anime as its own link, which still holds it...
    scene.world.tr.remove(&format!("{:040x}", 2));
    let holder = scene.rule(&first).await;
    scene
        .world
        .ctx
        .channels
        .delete_rule(&holder.id, holder.version)
        .await
        .unwrap();
    scene.link().await;
    assert_eq!(
        blocked_of(&scene.rule(&second).await),
        Some(format!("{work}:1").as_str()),
        "the season's link holds it"
    );

    // ...until the link is cut.
    scene
        .world
        .ctx
        .channels
        .set_season_anime(&work, 1, 1, None)
        .await
        .unwrap();
    scene.link().await;

    let released = scene.rule(&second).await;
    assert_eq!(season_of(&released), None);
    assert_eq!(blocked_of(&released), None, "nothing holds the season");
    assert!(released.version > noted.version);
    // Once cleared it stays as it is.
    scene.link().await;
    assert_eq!(scene.rule(&second).await.version, released.version);
}
