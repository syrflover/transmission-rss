//! A work's cover following its seasons' links (`docs/specs/library.md`, 시즌
//! 정보; ticket 0027) against a fake AniList ([`trss_anilist::fake`]), a
//! temporary app data folder and a clock the test moves.

use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use super::{queue::Ran, *};
use crate::{
    artwork::{image::samples, queue::Ran as ArtRan, AppData, Artwork},
    discovery::{Scan, ScannedWork, WorkRead},
    store::{
        artwork::{ArtworkError, JobKind, Mode, Note, Selection, Source, UserChange},
        library::LibraryStore,
    },
};
use trss_anilist::fake::Fake;

const HOUR: i64 = 60 * 60 * 1000;

struct Env {
    _dir: tempfile::TempDir,
    library: LibraryStore,
    art: Artwork,
    seasons: Seasons,
    fake: Fake,
    clock: Arc<AtomicI64>,
    folder: String,
}

fn scan(works: &[(&str, &[u32])]) -> Scan {
    Scan {
        works: works
            .iter()
            .map(|(name, seasons)| {
                WorkRead::Read(ScannedWork {
                    dir_name: (*name).to_owned(),
                    seasons: BTreeSet::from_iter(seasons.iter().copied()),
                    files: Vec::new(),
                    unrecognized: Vec::new(),
                })
            })
            .collect(),
    }
}

impl Env {
    async fn new(works: &[(&str, &[u32])]) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("trss.db")).await.unwrap();
        let fake = Fake::start().await;
        let clock = Arc::new(AtomicI64::new(1_000_000));
        let reading = clock.clone();
        let art = Artwork::with_clock(
            db.clone(),
            Some(AppData::new(dir.path())),
            fake.config(),
            Arc::new(move || reading.load(Ordering::SeqCst)),
        )
        .with_spacing(Duration::ZERO);
        let seasons = Seasons::over(db.clone(), &art);
        let library = LibraryStore::new(db);
        let (folder, _) = library
            .add_folder("/w".into(), scan(works), 100, &[])
            .await
            .unwrap();
        Env {
            _dir: dir,
            library,
            art,
            seasons,
            fake,
            clock,
            folder: folder.id,
        }
    }

    async fn id(&self, name: &str) -> String {
        self.library
            .works(&self.folder)
            .await
            .unwrap()
            .into_iter()
            .find(|w| w.dir_name == name)
            .unwrap()
            .id
    }

    /// AniList answers entry `id` (titled `title`), whose cover is `image`.
    fn serve(&self, id: i64, title: &str, image: &[u8]) {
        let mut state = self.fake.state.lock().unwrap();
        state.media.insert(id, self.fake.entry(id, title, &[]));
        state.images.insert(format!("{id}.jpg"), image.to_vec());
    }

    /// Links `ids` to the season as the user would, from its current version.
    async fn link(&self, work: &str, season: u32, ids: &[i64]) {
        let version = self.seasons.store.link(work, season).await.unwrap().version;
        self.seasons
            .set_links(work, season, version, ids.to_vec())
            .await
            .unwrap();
    }

    async fn selection(&self, work: &str) -> Selection {
        self.art.store.selection(work).await.unwrap()
    }

    /// The bytes of the work's cover.
    async fn cover(&self, work: &str) -> Vec<u8> {
        let image = self.selection(work).await.image.expect("an image");
        self.art.image(image).await.unwrap().to_vec()
    }

    /// Runs the cover jobs that are due until none is left.
    async fn drain(&self) -> Vec<ArtRan> {
        let mut ran = Vec::new();
        while let Some(r) = self.art.run_next().await {
            ran.push(r);
        }
        ran
    }

    /// Gives the work the cover of entry `id` as the follow does, and receives it.
    async fn follow(&self, work: &str, id: i64, image: &[u8]) {
        self.serve(id, "Show", image);
        self.link(work, 1, &[id]).await;
        assert_eq!(self.drain().await, [ArtRan::Recorded]);
    }

    fn searches(&self) -> usize {
        self.fake
            .api_requests()
            .iter()
            .filter(|(_, variables)| variables.get("search").is_some())
            .count()
    }
}

#[tokio::test]
async fn linking_the_first_season_of_an_undecided_cover_receives_that_entrys_cover() {
    let env = Env::new(&[("Show", &[1, 2])]).await;
    let id = env.id("Show").await;
    env.serve(10, "Show", &samples::jpeg());

    // The work's own search has not run: the cover is not decided.
    let before = env.selection(&id).await;
    assert_eq!((before.mode, before.source), (Mode::Auto, None));
    assert_eq!(before.job.as_ref().unwrap().kind, JobKind::Search);

    env.link(&id, 1, &[10]).await;
    // The entry is selected and its image asked for; nothing is shown before it is verified.
    let s = env.selection(&id).await;
    assert_eq!(
        (s.mode, s.source, s.anilist_media_id),
        (Mode::Auto, Some(Source::Anilist), Some(10))
    );
    assert!(s.image.is_none());
    assert_eq!(s.job.as_ref().unwrap().kind, JobKind::Fetch);
    assert!(s.version > before.version);

    assert_eq!(env.drain().await, [ArtRan::Recorded]);
    let s = env.selection(&id).await;
    assert_eq!((s.mode, s.job), (Mode::Auto, None));
    assert_eq!(env.cover(&id).await, samples::jpeg());
}

#[tokio::test]
async fn an_undecided_cover_with_a_note_is_taken_over_by_the_link() {
    // The automatic search found two entries with the folder's title and left the cover empty.
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.fake.add_search(
        "Show",
        vec![
            env.fake.entry(1, "Show", &[]),
            env.fake.entry(2, "Show", &[]),
        ],
        &samples::png(),
    );
    assert_eq!(env.drain().await, [ArtRan::Recorded]);
    assert_eq!(env.selection(&id).await.note, Some(Note::Ambiguous));

    env.link(&id, 1, &[2]).await;
    let s = env.selection(&id).await;
    assert_eq!((s.anilist_media_id, s.note), (Some(2), None));
    assert_eq!(env.drain().await, [ArtRan::Recorded]);
    assert_eq!(env.cover(&id).await, samples::png());
}

#[tokio::test]
async fn changing_the_first_entry_of_the_earliest_season_changes_the_cover_after_it_is_received() {
    let env = Env::new(&[("Show", &[1, 2])]).await;
    let id = env.id("Show").await;
    env.follow(&id, 10, &samples::jpeg()).await;
    let old = env.selection(&id).await;
    env.serve(20, "Show (other)", &samples::png());

    env.link(&id, 1, &[20]).await;
    // The entry followed is the new one, and the cover shown stays until its image is verified.
    let s = env.selection(&id).await;
    assert_eq!((s.mode, s.anilist_media_id), (Mode::Auto, Some(20)));
    assert_eq!(s.image, old.image);
    assert_eq!(s.job.as_ref().unwrap().kind, JobKind::Fetch);
    assert_eq!(env.cover(&id).await, samples::jpeg());

    assert_eq!(env.drain().await, [ArtRan::Recorded]);
    let s = env.selection(&id).await;
    assert_ne!(s.image, old.image);
    assert_eq!((s.mode, s.job), (Mode::Auto, None));
    assert_eq!(env.cover(&id).await, samples::png());

    // Still automatic: the next change is followed too, and so is an order that puts another entry first.
    env.serve(30, "Show (third)", &samples::webp());
    env.link(&id, 1, &[30, 20]).await;
    assert_eq!(env.drain().await, [ArtRan::Recorded]);
    assert_eq!(env.cover(&id).await, samples::webp());
}

#[tokio::test]
async fn a_link_that_leaves_the_earliest_seasons_first_entry_keeps_the_cover_as_it_is() {
    let env = Env::new(&[("Show", &[1, 2, 3])]).await;
    let id = env.id("Show").await;
    env.follow(&id, 10, &samples::jpeg()).await;
    let kept = env.selection(&id).await;
    for (id2, title) in [(11, "Show 2"), (12, "Show 1b")] {
        env.serve(id2, title, &samples::png());
    }

    // Season 2 linked for the first time.
    env.link(&id, 2, &[11]).await;
    assert_eq!(env.selection(&id).await, kept);
    // A second entry behind the first of season 1.
    env.link(&id, 1, &[10, 12]).await;
    assert_eq!(env.selection(&id).await, kept);
    // Season 2 unlinked again.
    env.link(&id, 2, &[]).await;
    assert_eq!(env.selection(&id).await, kept);

    assert_eq!(env.drain().await, []);
    assert!(!env.fake.image_asked("11.jpg") && !env.fake.image_asked("12.jpg"));
}

#[tokio::test]
async fn unlinking_the_earliest_season_passes_the_cover_to_the_next_linked_season() {
    let env = Env::new(&[("Show", &[1, 2])]).await;
    let id = env.id("Show").await;
    env.follow(&id, 10, &samples::jpeg()).await;
    env.serve(11, "Show 2", &samples::png());
    env.link(&id, 2, &[11]).await;

    env.link(&id, 1, &[]).await;
    assert_eq!(env.drain().await, [ArtRan::Recorded]);
    assert_eq!(env.selection(&id).await.anilist_media_id, Some(11));
    assert_eq!(env.cover(&id).await, samples::png());

    // Nothing is linked any more: nothing to follow, the cover stays.
    env.link(&id, 2, &[]).await;
    assert_eq!(env.drain().await, []);
    assert_eq!(env.selection(&id).await.anilist_media_id, Some(11));
    assert_eq!(env.cover(&id).await, samples::png());
}

#[tokio::test]
async fn the_specials_season_is_not_what_the_cover_follows() {
    let env = Env::new(&[("Show", &[0, 1])]).await;
    let id = env.id("Show").await;
    env.serve(30, "Show Specials", &samples::jpeg());
    let before = env.selection(&id).await;

    env.link(&id, 0, &[30]).await;
    assert_eq!(env.selection(&id).await, before);
    assert!(!env.fake.image_asked("30.jpg"));
}

#[tokio::test]
async fn a_picked_an_uploaded_or_a_cleared_cover_is_not_changed_by_a_link() {
    let env = Env::new(&[("Picked", &[1]), ("Uploaded", &[1]), ("Cleared", &[1])]).await;
    let (picked, uploaded, cleared) = (
        env.id("Picked").await,
        env.id("Uploaded").await,
        env.id("Cleared").await,
    );
    env.serve(40, "Picked", &samples::webp());
    env.serve(10, "Show", &samples::jpeg());

    let v = env.selection(&picked).await.version;
    env.art.pick(&picked, v, 40).await.unwrap();
    let v = env.selection(&uploaded).await.version;
    env.art
        .upload(&uploaded, v, samples::png(), None)
        .await
        .unwrap();
    let v = env.selection(&cleared).await.version;
    env.art
        .change(&cleared, v, UserChange::Clear)
        .await
        .unwrap();
    let before = [
        env.selection(&picked).await,
        env.selection(&uploaded).await,
        env.selection(&cleared).await,
    ];
    assert_eq!(
        before.each_ref().map(|s| s.mode),
        [Mode::Manual, Mode::Manual, Mode::Disabled]
    );

    for work in [&picked, &uploaded, &cleared] {
        env.link(work, 1, &[10]).await;
    }
    let after = [
        env.selection(&picked).await,
        env.selection(&uploaded).await,
        env.selection(&cleared).await,
    ];
    assert_eq!(after, before);
    assert_eq!(env.drain().await, []);
    assert!(!env.fake.image_asked("10.jpg"));
}

#[tokio::test]
async fn an_image_that_cannot_be_received_leaves_the_cover_and_the_saved_link() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.follow(&id, 10, &samples::jpeg()).await;
    let old = env.selection(&id).await;
    // The entry is there, its image host answers `404`.
    env.serve(20, "Show (other)", &samples::png());
    env.fake.state.lock().unwrap().images.remove("20.jpg");

    env.link(&id, 1, &[20]).await;
    // The link is saved whatever happens to the image.
    let link = env.seasons.store.link(&id, 1).await.unwrap();
    assert_eq!(link.entries.iter().map(|e| e.id).collect::<Vec<_>>(), [20]);

    // The receiving is tried again at the usual delays and then given up.
    let mut ran = Vec::new();
    for _ in 0..8 {
        env.clock.fetch_add(2 * HOUR, Ordering::SeqCst);
        match env.art.run_next().await {
            Some(r) => ran.push(r),
            None => break,
        }
    }
    assert_eq!(
        ran,
        [
            ArtRan::Later,
            ArtRan::Later,
            ArtRan::Later,
            ArtRan::GaveUp(Note::Failed)
        ]
    );
    // The cover is the old one, and the reason is where the cover view reads it.
    let s = env.selection(&id).await;
    assert_eq!((s.job, s.note), (None, Some(Note::Failed)));
    assert_eq!(s.image, old.image);
    assert_eq!(env.cover(&id).await, samples::jpeg());
}

#[tokio::test]
async fn an_image_that_is_not_an_image_is_refused_and_the_cover_stays() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.follow(&id, 10, &samples::jpeg()).await;
    env.serve(20, "Show (other)", b"this is not an image");

    env.link(&id, 1, &[20]).await;
    assert_eq!(env.drain().await, [ArtRan::GaveUp(Note::Rejected)]);
    let s = env.selection(&id).await;
    assert_eq!((s.job, s.note), (None, Some(Note::Rejected)));
    assert_eq!(env.cover(&id).await, samples::jpeg());
}

#[tokio::test]
async fn a_late_automatic_image_never_undoes_the_users_choice() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.follow(&id, 10, &samples::jpeg()).await;
    env.serve(20, "Show (other)", &samples::png());
    env.link(&id, 1, &[20]).await;

    // The image of the followed entry is on its way ...
    let seen = env.selection(&id).await;
    env.fake.hold("20.jpg");
    let art = env.art.clone();
    let task = tokio::spawn(async move { art.run_next().await });
    for _ in 0..200 {
        if env.fake.image_asked("20.jpg") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(env.fake.image_asked("20.jpg"));

    // ... when the user uploads their own, and a screen that still shows the
    // version before the link is refused.
    let uploaded = env
        .art
        .upload(&id, seen.version, samples::webp(), None)
        .await
        .unwrap();
    env.fake.release("20.jpg");
    assert_eq!(task.await.unwrap(), Some(ArtRan::Dropped));
    assert_eq!(env.selection(&id).await, uploaded);
    assert_eq!(uploaded.mode, Mode::Manual);
    assert_eq!(env.cover(&id).await, samples::webp());
    assert!(matches!(
        env.art
            .upload(&id, seen.version - 1, samples::png(), None)
            .await,
        Err(crate::artwork::ActionError::Store(ArtworkError::Conflict(
            _
        )))
    ));

    // A link saved afterwards does not take the user's cover back.
    env.link(&id, 1, &[10]).await;
    assert_eq!(env.selection(&id).await, uploaded);
}

#[tokio::test]
async fn the_apps_own_link_of_a_first_season_makes_the_cover_follow_it_too() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.fake.add_search(
        "Show",
        vec![env.fake.entry(10, "Show", &[])],
        &samples::jpeg(),
    );

    // The season's search links the entry; the cover's own search never has to run.
    assert_eq!(env.seasons.run_next().await, Some(Ran::Linked(10)));
    let s = env.selection(&id).await;
    assert_eq!(
        (s.mode, s.anilist_media_id, s.job.map(|j| j.kind)),
        (Mode::Auto, Some(10), Some(JobKind::Fetch))
    );
    assert_eq!(env.drain().await, [ArtRan::Recorded]);
    assert_eq!(env.cover(&id).await, samples::jpeg());
    assert_eq!(env.searches(), 1);
}

#[tokio::test]
async fn asking_the_first_season_to_be_found_again_lets_the_cover_follow_what_is_left() {
    let env = Env::new(&[("Show", &[1, 2])]).await;
    let id = env.id("Show").await;
    env.follow(&id, 10, &samples::jpeg()).await;
    env.serve(11, "Show 2", &samples::png());
    env.link(&id, 2, &[11]).await;

    let version = env.seasons.store.link(&id, 1).await.unwrap().version;
    env.seasons.restart_auto(&id, 1, version).await.unwrap();
    assert_eq!(env.selection(&id).await.anilist_media_id, Some(11));
    assert_eq!(env.drain().await, [ArtRan::Recorded]);
    assert_eq!(env.cover(&id).await, samples::png());
}
