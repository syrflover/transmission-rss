//! The artwork flow end to end against a fake AniList ([`super::fake`]) and a
//! temporary app data folder: the rows of `docs/specs/library.md` 작품 표지 and
//! of ticket 0015, except the YAML import (goal 5).

use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};

use serde_json::json;
use tempfile::TempDir;

use super::{
    fake::Fake,
    files::{self, AppData, ARTWORK_DIR, STAGING_DIR, STALE_STAGING},
    image::samples,
    queue::Ran,
    *,
};
use crate::{
    discovery::{Scan, ScannedWork, WorkRead},
    store::{
        artwork::{JobKind, Mode, Note, Selection, Source, UserChange},
        library::{LibraryStore, ListQuery},
        DbError,
    },
};

struct Env {
    dir: TempDir,
    db: Db,
    library: LibraryStore,
    art: Artwork,
    fake: Fake,
    folder: String,
}

fn read(name: &str) -> WorkRead {
    WorkRead::Read(ScannedWork {
        dir_name: name.to_owned(),
        seasons: BTreeSet::new(),
        files: Vec::new(),
        unrecognized: Vec::new(),
    })
}

fn scan(names: &[&str]) -> Scan {
    Scan {
        works: names.iter().map(|n| read(n)).collect(),
    }
}

impl Env {
    async fn new(names: &[&str]) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("trss.db")).await.unwrap();
        let fake = Fake::start().await;
        let art = Artwork::new(db.clone(), Some(AppData::new(dir.path())), fake.config())
            .with_spacing(Duration::ZERO);
        let library = LibraryStore::new(db.clone());
        let (folder, _) = library
            .add_folder("/w".into(), scan(names), 100, &[])
            .await
            .unwrap();
        Env {
            dir,
            db,
            library,
            art,
            fake,
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

    async fn selection(&self, name: &str) -> Selection {
        self.art
            .store
            .selection(&self.id(name).await)
            .await
            .unwrap()
    }

    /// Runs due jobs until none is left.
    async fn drain(&self) -> Vec<Ran> {
        let mut ran = Vec::new();
        while let Some(r) = self.art.run_next().await {
            ran.push(r);
        }
        ran
    }

    /// The files in the artwork folder (not the staging folder), by name.
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.dir.path().join(ARTWORK_DIR))
            .map(|entries| {
                entries
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .filter(|n| n != ".staging")
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn staging(&self) -> Vec<String> {
        fs::read_dir(self.dir.path().join(STAGING_DIR))
            .map(|entries| {
                entries
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    async fn sql(&self, sql: &'static str, id: String) {
        self.db
            .run::<_, DbError, _>(move |c| Ok(c.execute(sql, [id]).map(|_| ())?))
            .await
            .unwrap();
    }
}

fn image_of(selection: &Selection) -> &crate::store::artwork::ImageRef {
    selection.image.as_ref().expect("an image")
}

// --- the automatic decision -------------------------------------------------------------

#[tokio::test]
async fn one_exact_title_is_selected_and_shown_only_after_its_bytes_are_verified() {
    let env = Env::new(&["Lycoris Recoil"]).await;
    let jpeg = samples::jpeg();
    env.fake.add_search(
        "Lycoris Recoil",
        vec![
            env.fake
                .entry(2, "Lycoris Recoil: Friends are thieves of time.", &[]),
            env.fake.entry(143270, "Lycoris Recoil", &[]),
            env.fake.entry(3, "Lycoris Recoil Season 2", &[]),
        ],
        &jpeg,
    );

    // The search selects the ID; no image is shown before it is verified.
    assert_eq!(env.art.run_next().await, Some(Ran::Recorded));
    let s = env.selection("Lycoris Recoil").await;
    assert_eq!(
        (s.mode, s.source, s.anilist_media_id),
        (Mode::Auto, Some(Source::Anilist), Some(143270))
    );
    assert!(s.image.is_none());
    assert_eq!(s.job.as_ref().unwrap().kind, JobKind::Fetch);

    assert_eq!(env.art.run_next().await, Some(Ran::Recorded));
    let s = env.selection("Lycoris Recoil").await;
    let image = image_of(&s);
    assert_eq!(
        (image.origin, image.format),
        (Source::Anilist, Format::Jpeg)
    );
    assert!(s.job.is_none());
    assert_eq!(env.art.image(image.clone()).await.unwrap(), jpeg);
    assert_eq!(env.files().len(), 1);
    assert!(env.staging().is_empty());
    assert_eq!(env.art.run_next().await, None);
}

#[tokio::test]
async fn a_work_of_several_seasons_takes_the_entry_named_like_its_folder() {
    // Several local seasons alone do not hold the automatic cover back: the
    // entry whose title is the folder name (usually the first season) is used.
    let env = Env::new(&[]).await;
    let work = ScannedWork {
        dir_name: "Kaguya-sama wa Kokurasetai".to_owned(),
        seasons: BTreeSet::from([1, 2, 3]),
        files: Vec::new(),
        unrecognized: Vec::new(),
    };
    env.library
        .record_scan(
            &env.folder,
            Ok(Scan {
                works: vec![WorkRead::Read(work)],
            }),
            200,
        )
        .await
        .unwrap();
    let seasons = env.library.works(&env.folder).await.unwrap()[0]
        .seasons
        .clone();
    assert_eq!(seasons, [1, 2, 3]);
    env.fake.add_search(
        "Kaguya-sama wa Kokurasetai",
        vec![
            env.fake.entry(101921, "Kaguya-sama wa Kokurasetai", &[]),
            env.fake.entry(
                112641,
                "Kaguya-sama wa Kokurasetai?: Tensai-tachi no Renai Zunousen",
                &[],
            ),
            env.fake
                .entry(125367, "Kaguya-sama wa Kokurasetai: Ultra Romantic", &[]),
        ],
        &samples::jpeg(),
    );
    assert_eq!(env.drain().await, [Ran::Recorded, Ran::Recorded]);
    let s = env.selection("Kaguya-sama wa Kokurasetai").await;
    assert_eq!(
        (s.mode, s.source, s.anilist_media_id),
        (Mode::Auto, Some(Source::Anilist), Some(101921))
    );
    assert!(s.image.is_some());
}

#[tokio::test]
async fn an_image_url_that_is_gone_is_asked_for_again_by_the_entrys_id() {
    let env = Env::new(&["A"]).await;
    env.fake
        .add_search("A", vec![env.fake.entry(1, "A", &[])], &samples::jpeg());
    // The search selects; the image is not fetched yet.
    assert_eq!(env.art.run_next().await, Some(Ran::Recorded));
    assert_eq!(env.selection("A").await.job.unwrap().kind, JobKind::Fetch);

    // AniList moved the cover: the old URL answers 404, the entry the new one.
    {
        let mut state = env.fake.state.lock().unwrap();
        state.images.remove("1.jpg");
        state.images.insert("1-moved.png".into(), samples::png());
        let moved = format!("{}/img/1-moved.png", env.fake.origin);
        let entry = state.media.get_mut(&1).unwrap();
        entry["coverImage"]["extraLarge"] = moved.clone().into();
        entry["coverImage"]["large"] = moved.into();
    }
    let requests = env.fake.api_requests().len();
    assert_eq!(env.art.run_next().await, Some(Ran::Recorded));
    let a = env.selection("A").await;
    assert_eq!(
        env.art.image(image_of(&a).clone()).await.unwrap(),
        samples::png()
    );
    let asked: Vec<_> = env.fake.api_requests()[requests..]
        .iter()
        .map(|(_, v)| v.clone())
        .collect();
    assert_eq!(asked, [json!({ "id": 1 })]);
}

#[tokio::test]
async fn an_image_that_is_gone_where_the_entry_still_points_waits_like_a_failure() {
    let env = Env::new(&["B"]).await;
    env.fake
        .add_search("B", vec![env.fake.entry(2, "B", &[])], &samples::jpeg());
    assert_eq!(env.art.run_next().await, Some(Ran::Recorded));
    env.fake.state.lock().unwrap().images.remove("2.jpg");
    assert_eq!(env.art.run_next().await, Some(Ran::Later));
    let job = env.selection("B").await.job.unwrap();
    assert_eq!((job.kind, job.attempts), (JobKind::Fetch, 1));
    // The URL is fetched once in the run, not again after the entry named it.
    let state = env.fake.state.lock().unwrap();
    let tries = state
        .image_requests
        .iter()
        .filter(|n| *n == "2.jpg")
        .count();
    assert_eq!(tries, 1);
}

#[tokio::test]
async fn an_entry_whose_image_is_not_an_image_selects_the_id_but_shows_nothing() {
    let env = Env::new(&["Lycoris Recoil"]).await;
    env.fake.add_search(
        "Lycoris Recoil",
        vec![env.fake.entry(1, "Lycoris Recoil", &[])],
        b"<html>not an image</html>",
    );
    assert_eq!(
        env.drain().await,
        [Ran::Recorded, Ran::GaveUp(Note::Rejected)]
    );
    let s = env.selection("Lycoris Recoil").await;
    assert_eq!(s.anilist_media_id, Some(1));
    assert!(s.image.is_none());
    assert_eq!(s.note, Some(Note::Rejected));
    assert!(env.files().is_empty());
    assert!(env.staging().is_empty());
}

#[tokio::test]
async fn two_entries_with_the_same_title_leave_the_cover_empty_and_the_work_as_it_is() {
    let env = Env::new(&["Lycoris Recoil", "Clevatess"]).await;
    env.fake.add_search(
        "Lycoris Recoil",
        vec![
            env.fake.entry(1, "Lycoris Recoil", &[]),
            // The second season listed under the same name.
            env.fake
                .entry(3, "Lycoris Recoil Season 2", &["Lycoris Recoil"]),
        ],
        &samples::jpeg(),
    );
    env.fake.add_search("Clevatess", vec![], &[]);
    env.drain().await;
    let s = env.selection("Lycoris Recoil").await;
    assert_eq!(
        (s.mode, s.source, s.anilist_media_id, s.note),
        (Mode::Auto, None, None, Some(Note::Ambiguous))
    );
    assert_eq!(env.selection("Clevatess").await.note, Some(Note::NoMatch));
    // The work is listed as before, and the user can still choose.
    let page = env
        .library
        .list(ListQuery {
            sort: crate::store::library::Sort::Title,
            filter: crate::store::library::Filter::All,
            search: String::new(),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    let picked = env
        .art
        .pick(&env.id("Lycoris Recoil").await, s.version, 1)
        .await
        .unwrap();
    assert_eq!(
        (picked.mode, picked.anilist_media_id),
        (Mode::Manual, Some(1))
    );
    assert!(env.art.image(picked.image.unwrap()).await.is_ok());
    assert!(
        !env.fake.image_asked("3.jpg"),
        "only the picked entry's image is fetched"
    );
}

#[tokio::test]
async fn a_search_counts_only_when_read_to_its_last_page() {
    let env = Env::new(&["A", "B", "C", "A 2"]).await;
    let filler = |from: i64| -> Vec<serde_json::Value> {
        (from..from + 3)
            .map(|id| env.fake.entry(id, &format!("Other {id}"), &[]))
            .collect()
    };
    {
        let mut state = env.fake.state.lock().unwrap();
        // A: the match is on the second and last page.
        state.searches.insert(
            "A".into(),
            vec![filler(100), vec![env.fake.entry(1, "a", &[])]],
        );
        // B: one match on page 1, but more pages than the app reads.
        let mut pages = vec![vec![env.fake.entry(2, "B", &[])]];
        for p in 0..anilist::MAX_SEARCH_PAGES as i64 {
            pages.push(filler(200 + p * 10));
        }
        state.searches.insert("B".into(), pages);
        // C: a namesake on the next page.
        state.searches.insert(
            "C".into(),
            vec![
                vec![env.fake.entry(3, "C", &[])],
                vec![env.fake.entry(4, "Other", &["c"])],
            ],
        );
        // "A 2": similar titles only (a season number is no evidence).
        state.searches.insert(
            "A 2".into(),
            vec![vec![
                env.fake.entry(1, "a", &[]),
                env.fake.entry(5, "A Season 2", &["A 2nd Season"]),
            ]],
        );
        state.media.insert(1, env.fake.entry(1, "a", &[]));
        state.images.insert("1.jpg".into(), samples::png());
    }
    env.drain().await;
    let s = env.selection("A").await;
    assert_eq!(s.anilist_media_id, Some(1));
    assert_eq!(image_of(&s).format, Format::Png);
    assert_eq!(env.selection("B").await.note, Some(Note::Incomplete));
    assert_eq!(env.selection("B").await.source, None);
    assert_eq!(env.selection("C").await.note, Some(Note::Ambiguous));
    assert_eq!(env.selection("A 2").await.note, Some(Note::NoMatch));
}

// --- pace, failures and the queue --------------------------------------------------------

#[tokio::test]
async fn a_failure_waits_and_a_429_holds_every_request_without_counting() {
    let env = Env::new(&["A"]).await;
    env.fake
        .add_search("A", vec![env.fake.entry(1, "A", &[])], &samples::jpeg());
    let id = env.id("A").await;

    env.fake.state.lock().unwrap().failing = 1;
    let before = env.art.now();
    assert_eq!(env.art.run_next().await, Some(Ran::Later));
    let job = env.selection("A").await.job.unwrap();
    assert_eq!(job.attempts, 1);
    let wait = job.not_before.unwrap() - before;
    assert!((60_000..65_000).contains(&wait), "{wait}");
    // Not due yet.
    assert_eq!(env.art.run_next().await, None);

    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = 30;
    }
    let claimed = env.art.store.next_job(i64::MAX).await.unwrap().unwrap();
    assert_eq!(env.art.run_job(&claimed).await, Ran::Later);
    let job = env.selection("A").await.job.unwrap();
    assert_eq!(job.attempts, 1, "a 429 is not a failure");
    let now = env.art.now();
    // Every request, the web's included, waits for the block.
    let slot = env
        .art
        .store
        .take_request_slot(now, 0, Some(1000))
        .await
        .unwrap();
    assert!(matches!(slot, Err(wait) if wait > 25_000), "{slot:?}");
    assert!(matches!(
        env.art
            .anilist
            .search_page("A", 1, Some(Duration::from_secs(1)))
            .await,
        Err(AnilistError::Busy { .. })
    ));

    // Three failures in all, then the job is given up and the user is told.
    env.sql(
        "UPDATE anilist_pace SET next_at = 0, blocked_until = NULL WHERE ?1 = ?1",
        id.clone(),
    )
    .await;
    env.fake.state.lock().unwrap().failing = 10;
    for _ in 0..2 {
        let claimed = env.art.store.next_job(i64::MAX).await.unwrap().unwrap();
        assert_eq!(env.art.run_job(&claimed).await, Ran::Later);
    }
    let claimed = env.art.store.next_job(i64::MAX).await.unwrap().unwrap();
    assert_eq!(claimed.attempts, 3);
    assert_eq!(env.art.run_job(&claimed).await, Ran::GaveUp(Note::Failed));
    let s = env.selection("A").await;
    assert_eq!((s.job, s.note, s.source), (None, Some(Note::Failed), None));
}

/// Makes the writes `sql` names fail, as a full disk or a broken file would.
async fn refuse(env: &Env, sql: &'static str) {
    env.db
        .run::<_, DbError, _>(move |c| Ok(c.execute_batch(sql)?))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_outcome_that_cannot_be_written_waits_like_a_failure_instead_of_asking_again_at_once() {
    let env = Env::new(&["A"]).await;
    env.fake
        .add_search("A", vec![env.fake.entry(1, "A", &[])], &samples::jpeg());

    // The search's answer cannot be recorded.
    refuse(
        &env,
        "CREATE TRIGGER refuse BEFORE UPDATE OF anilist_media_id ON work_artwork
         WHEN NEW.anilist_media_id IS NOT NULL
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .await;
    let before = env.art.now();
    assert_eq!(env.art.run_next().await, Some(Ran::Later));
    let job = env.selection("A").await.job.unwrap();
    assert_eq!((job.kind, job.attempts), (JobKind::Search, 1));
    let wait = job.not_before.unwrap() - before;
    assert!((60_000..65_000).contains(&wait), "{wait}");
    let requests = env.fake.api_requests().len();
    assert_eq!(env.art.run_next().await, None);
    assert_eq!(env.fake.api_requests().len(), requests);
    refuse(&env, "DROP TRIGGER refuse;").await;
    let claimed = env.art.store.next_job(i64::MAX).await.unwrap().unwrap();
    assert_eq!(env.art.run_job(&claimed).await, Ran::Recorded);

    // Nor can the fetched image.
    refuse(
        &env,
        "CREATE TRIGGER refuse BEFORE UPDATE OF image_id ON work_artwork
         WHEN NEW.image_id IS NOT NULL
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .await;
    let before = env.art.now();
    assert_eq!(env.art.run_next().await, Some(Ran::Later));
    let s = env.selection("A").await;
    let job = s.job.unwrap();
    assert_eq!((job.kind, job.attempts), (JobKind::Fetch, 1));
    let wait = job.not_before.unwrap() - before;
    assert!((60_000..65_000).contains(&wait), "{wait}");
    assert!(s.image.is_none());
    assert_eq!(env.art.run_next().await, None);
    refuse(&env, "DROP TRIGGER refuse;").await;
    let claimed = env.art.store.next_job(i64::MAX).await.unwrap().unwrap();
    assert_eq!(env.art.run_job(&claimed).await, Ran::Recorded);
    let s = env.selection("A").await;
    assert_eq!(
        env.art.image(image_of(&s).clone()).await.unwrap(),
        samples::jpeg()
    );
}

#[tokio::test]
async fn an_answer_larger_than_the_limit_is_refused_before_it_is_read_whole() {
    let env = Env::new(&["A"]).await;
    env.fake
        .add_search("A", vec![env.fake.entry(1, "A", &[])], &samples::jpeg());
    let limit = super::anilist::MAX_ANSWER_BYTES;
    for chunked in [false, true] {
        {
            let mut state = env.fake.state.lock().unwrap();
            state.chunked = chunked;
            state.padding = limit - 4096;
        }
        let page = env.art.anilist.search_page("A", 1, None).await.unwrap();
        assert_eq!(page.candidates.len(), 1, "chunked: {chunked}");
        env.fake.state.lock().unwrap().padding = limit;
        assert!(
            matches!(
                env.art.anilist.search_page("A", 1, None).await,
                Err(AnilistError::Invalid(_))
            ),
            "chunked: {chunked}"
        );
        assert!(matches!(
            env.art.anilist.media(1, None).await,
            Err(AnilistError::Invalid(_))
        ));
    }
}

#[tokio::test]
async fn hundreds_of_new_works_are_searched_one_at_a_time_at_the_pace() {
    let names: Vec<String> = (0..520).map(|i| format!("Work {i:03}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let env = Env::new(&refs).await;
    let spacing = Duration::from_millis(40);

    // Two processes (two connections to one database) share one pace.
    let other_db = Db::open(env.dir.path().join("trss.db")).await.unwrap();
    let other = Artwork::new(
        other_db,
        Some(AppData::new(env.dir.path())),
        env.fake.config(),
    )
    .with_spacing(spacing);
    let art = env.art.clone().with_spacing(spacing);

    let pending = env
        .db
        .run::<_, DbError, _>(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM work_artwork WHERE job = 'search'",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(pending, 520);

    let a = tokio::spawn({
        let art = art.clone();
        async move {
            for _ in 0..4 {
                art.run_next().await;
            }
        }
    });
    let b = tokio::spawn({
        let other = other.clone();
        async move {
            for _ in 0..4 {
                other.run_next().await;
            }
        }
    });
    // The library answers while the queue runs.
    let started = Instant::now();
    let page = env
        .library
        .list(ListQuery {
            sort: crate::store::library::Sort::Title,
            filter: crate::store::library::Filter::All,
            search: String::new(),
            after: None,
            limit: 60,
        })
        .await
        .unwrap();
    assert_eq!(page.library_count, 520);
    assert!(started.elapsed() < Duration::from_secs(2));
    a.await.unwrap();
    b.await.unwrap();

    let mut times: Vec<Instant> = env
        .fake
        .api_requests()
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    times.sort();
    assert!(times.len() >= 6, "{}", times.len());
    for pair in times.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(gap >= spacing - Duration::from_millis(5), "{gap:?}");
    }
    // Jobs ran oldest first; the rest wait, none was lost.
    let left = env
        .db
        .run::<_, DbError, _>(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM work_artwork WHERE job IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap();
    assert!(left >= 512, "{left}");
    // The queue's lock is not the cycle's.
    assert_ne!(
        queue::lock_path_for(&env.dir.path().join("trss.db")),
        crate::worker::lock_path_for(&env.dir.path().join("trss.db"))
    );
}

#[tokio::test]
async fn the_queue_stops_on_shutdown_and_a_restart_resumes_it() {
    let env = Env::new(&["A", "B"]).await;
    env.fake.add_search("A", vec![], &[]);
    env.fake.add_search("B", vec![], &[]);
    let cancel = tokio_util::sync::CancellationToken::new();
    let lock = queue::lock_path_for(&env.dir.path().join("trss.db"));
    let art = env.art.clone().with_spacing(Duration::from_secs(3600));
    let run = tokio::spawn({
        let (art, lock, cancel) = (art.clone(), lock.clone(), cancel.clone());
        async move { art.run_queue(lock, cancel).await }
    });
    // The first search goes out at once; the second waits its turn, an hour.
    for _ in 0..100 {
        if !env.fake.api_requests().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap();
    // One search ran (which one is up to the works' random IDs); the other is
    // still asked for.
    let (a, b) = (env.selection("A").await, env.selection("B").await);
    let (done, waiting) = if a.job.is_none() { (a, b) } else { (b, a) };
    assert_eq!(done.note, Some(Note::NoMatch));
    assert!(waiting.job.is_some(), "still asked for");

    // The next start runs it.
    env.sql(
        "UPDATE anilist_pace SET next_at = 0 WHERE ?1 = ?1",
        String::new(),
    )
    .await;
    assert_eq!(env.drain().await, [Ran::Recorded]);
    assert_eq!(
        env.art
            .store
            .selection(&waiting.work_id)
            .await
            .unwrap()
            .note,
        Some(Note::NoMatch)
    );
}

// --- user choices and late automatic results --------------------------------------------

/// Runs the search of `name` (one exact match, entry `id`) and holds the
/// fetch of its image, which runs in a task.
async fn held_fetch(env: &Env, name: &str, id: i64) -> tokio::task::JoinHandle<Option<Ran>> {
    env.fake
        .add_search(name, vec![env.fake.entry(id, name, &[])], &samples::jpeg());
    let image = format!("{id}.jpg");
    env.fake.hold(&image);
    // The search of this work (the queue's order among works recorded in the
    // same millisecond is by their random IDs).
    let s = env.selection(name).await;
    let claimed = crate::store::artwork::ClaimedJob {
        work_id: env.id(name).await,
        dir_name: name.to_owned(),
        version: s.version,
        requested_at: s.job.as_ref().unwrap().requested_at,
        kind: JobKind::Search,
        anilist_media_id: None,
        image_url: None,
        attempts: 0,
    };
    assert_eq!(env.art.run_job(&claimed).await, Ran::Recorded);
    let art = env.art.clone();
    let task = tokio::spawn(async move { art.run_next().await });
    for _ in 0..200 {
        if env.fake.image_asked(&image) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(env.fake.image_asked(&image));
    task
}

#[tokio::test]
async fn a_late_automatic_image_never_undoes_an_upload_a_pick_or_a_clear() {
    let env = Env::new(&["Up", "Pick", "Clear"]).await;

    // Upload while the automatic image is on its way.
    let task = held_fetch(&env, "Up", 1).await;
    let s = env.selection("Up").await;
    let uploaded = env
        .art
        .upload(&env.id("Up").await, s.version, samples::png(), None)
        .await
        .unwrap();
    env.fake.release("1.jpg");
    assert_eq!(task.await.unwrap(), Some(Ran::Dropped));
    assert_eq!(env.selection("Up").await, uploaded);
    assert_eq!(uploaded.source, Some(Source::Upload));

    // Pick another entry meanwhile.
    let task = held_fetch(&env, "Pick", 2).await;
    {
        let mut state = env.fake.state.lock().unwrap();
        state
            .media
            .insert(20, env.fake.entry(20, "Pick (2009)", &[]));
        state.images.insert("20.jpg".into(), samples::webp());
    }
    let s = env.selection("Pick").await;
    let picked = env
        .art
        .pick(&env.id("Pick").await, s.version, 20)
        .await
        .unwrap();
    env.fake.release("2.jpg");
    assert_eq!(task.await.unwrap(), Some(Ran::Dropped));
    let now = env.selection("Pick").await;
    assert_eq!(now, picked);
    assert_eq!(
        (now.mode, now.anilist_media_id, image_of(&now).format),
        (Mode::Manual, Some(20), Format::Webp)
    );

    // Clear meanwhile.
    let task = held_fetch(&env, "Clear", 3).await;
    let s = env.selection("Clear").await;
    env.art
        .change(&env.id("Clear").await, s.version, UserChange::Clear)
        .await
        .unwrap();
    env.fake.release("3.jpg");
    assert_eq!(task.await.unwrap(), Some(Ran::Dropped));
    let now = env.selection("Clear").await;
    assert_eq!(
        (now.mode, now.source, now.job),
        (Mode::Disabled, None, None)
    );
    // ... and nothing searches for it again.
    assert_eq!(env.drain().await, []);

    // Only the two chosen images remain; the dropped ones were cleaned up.
    assert_eq!(env.files().len(), 2);
    assert!(env.staging().is_empty());
}

#[tokio::test]
async fn an_image_that_arrives_first_does_not_stop_a_choice_made_before_it() {
    let env = Env::new(&["A"]).await;
    let task = held_fetch(&env, "A", 1).await;
    let seen = env.selection("A").await;
    env.fake.release("1.jpg");
    assert_eq!(task.await.unwrap(), Some(Ran::Recorded));
    // The screen still shows the version before the image; the user's upload
    // from it applies (the arrival changed no choice).
    let uploaded = env
        .art
        .upload(&env.id("A").await, seen.version, samples::png(), None)
        .await
        .unwrap();
    assert_eq!(uploaded.source, Some(Source::Upload));
    assert_eq!(env.files().len(), 1);
}

#[tokio::test]
async fn of_two_screens_changing_from_the_same_version_the_first_stays() {
    let env = Env::new(&["A"]).await;
    let id = env.id("A").await;
    let v = env.selection("A").await.version;
    let first = env.art.upload(&id, v, samples::png(), None).await.unwrap();
    match env.art.upload(&id, v, samples::jpeg(), None).await {
        Err(ActionError::Store(ArtworkError::Conflict(current))) => assert_eq!(*current, first),
        other => panic!("expected a conflict, got {other:?}"),
    }
    match env.art.change(&id, v, UserChange::Clear).await {
        Err(ActionError::Store(ArtworkError::Conflict(_))) => {}
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(env.selection("A").await, first);
    assert_eq!(env.files().len(), 1);
}

#[tokio::test]
async fn an_upload_is_judged_by_its_bytes_and_a_refusal_keeps_the_cover() {
    let env = Env::new(&["A"]).await;
    let id = env.id("A").await;
    let v = env.selection("A").await.version;
    // A JPEG (whatever it was called) is a JPEG.
    let kept = env.art.upload(&id, v, samples::jpeg(), None).await.unwrap();
    assert_eq!(image_of(&kept).format, Format::Jpeg);
    let file = env.files();

    let mut too_big = samples::png();
    too_big.resize(MAX_IMAGE_BYTES + 1, 0);
    let jpeg = samples::jpeg();
    for (bytes, why) in [
        (b"just text, named .jpg".to_vec(), Rejected::NotImage),
        (too_big, Rejected::TooLarge),
        (samples::png_claiming(5000, 5000), Rejected::TooManyPixels),
        (jpeg[..jpeg.len() / 2].to_vec(), Rejected::Damaged),
        (samples::gif(), Rejected::NotImage),
    ] {
        match env.art.upload(&id, kept.version, bytes, None).await {
            Err(ActionError::Rejected(r)) => assert_eq!(r, why),
            other => panic!("expected {why:?}, got {other:?}"),
        }
        assert_eq!(env.selection("A").await, kept);
        assert_eq!(env.files(), file);
        assert!(env.staging().is_empty());
    }
}

// --- files --------------------------------------------------------------------------------

#[tokio::test]
async fn an_interrupted_publish_is_finished_by_the_files_identity() {
    let env = Env::new(&["A"]).await;
    let app = AppData::new(env.dir.path());
    let before = env.selection("A").await;

    // Stopped after the rename, before a selection took it.
    files::publish_at(
        &app,
        &env.art.store,
        samples::png().into(),
        "artwork/x.png",
        "artwork/.staging/x.tmp",
        1_000,
    )
    .await
    .unwrap();
    // Stopped after writing the staged file, before the rename.
    env.art
        .store
        .reserve_file("artwork/y.png", "artwork/.staging/y.tmp", 1_000)
        .await
        .unwrap();
    fs::write(env.path("artwork/.staging/y.tmp"), samples::png()).unwrap();
    let meta = fs::metadata(env.path("artwork/.staging/y.tmp")).unwrap();
    use std::os::unix::fs::MetadataExt;
    env.art
        .store
        .file_identity("artwork/y.png", meta.dev(), meta.ino())
        .await
        .unwrap();
    // Stopped before its identity was recorded: the staged file cannot be told
    // from someone else's, so it stays.
    env.art
        .store
        .reserve_file("artwork/z.png", "artwork/.staging/z.tmp", 1_000)
        .await
        .unwrap();
    fs::write(env.path("artwork/.staging/z.tmp"), b"?").unwrap();
    // A file the app never recorded.
    fs::write(env.path("artwork/stranger.png"), samples::png()).unwrap();

    // A recent publish may still be running: left alone.
    assert_eq!(
        files::recover(&app, &env.art.store, 2_000).await.unwrap(),
        0
    );
    let late = 1_000 + STALE_STAGING.as_millis() as i64 + 1;
    assert_eq!(files::recover(&app, &env.art.store, late).await.unwrap(), 3);
    env.art.tidy().await;
    assert_eq!(env.files(), ["stranger.png"]);
    assert_eq!(env.staging(), ["z.tmp"]);
    assert_eq!(env.selection("A").await, before);
}

#[tokio::test]
async fn a_taken_place_is_never_overwritten() {
    let env = Env::new(&["A"]).await;
    let app = AppData::new(env.dir.path());
    fs::create_dir_all(env.path("artwork")).unwrap();
    fs::write(env.path("artwork/taken.png"), b"someone else's").unwrap();
    let result = files::publish_at(
        &app,
        &env.art.store,
        samples::png().into(),
        "artwork/taken.png",
        "artwork/.staging/taken.tmp",
        1,
    )
    .await;
    assert!(matches!(result, Err(files::PublishError::Occupied)));
    assert_eq!(
        fs::read(env.path("artwork/taken.png")).unwrap(),
        b"someone else's"
    );
    assert!(env.staging().is_empty());
    let rows = env
        .art
        .store
        .run(|c| Ok(crate::store::artwork::files_of_state(c, "staging")?))
        .await
        .unwrap();
    assert!(rows.is_empty());
    env.art.tidy().await;
    assert_eq!(env.files(), ["taken.png"]);
}

#[tokio::test]
async fn an_image_is_served_only_while_its_file_is_the_recorded_one() {
    let env = Env::new(&["A"]).await;
    let id = env.id("A").await;
    let v = env.selection("A").await.version;
    let s = env.art.upload(&id, v, samples::png(), None).await.unwrap();
    let image = image_of(&s).clone();
    let path = env.path(&image.relative_path);
    let good = fs::read(&path).unwrap();
    assert_eq!(env.art.image(image.clone()).await, Ok(good.clone()));

    // Other bytes of the same size, a disguised text, a cut file.
    let mut other = good.clone();
    let last = other.len() - 1;
    other[last] ^= 0xff;
    fs::write(&path, &other).unwrap();
    assert_eq!(
        env.art.image(image.clone()).await,
        Err(Unavailable::Mismatch)
    );
    fs::write(&path, vec![b'x'; good.len()]).unwrap();
    assert_eq!(
        env.art.image(image.clone()).await,
        Err(Unavailable::Mismatch)
    );
    fs::write(&path, &good[..10]).unwrap();
    assert_eq!(
        env.art.image(image.clone()).await,
        Err(Unavailable::Mismatch)
    );

    // A link to the right bytes, a linked folder, a path leading out.
    fs::write(env.path("copy.png"), &good).unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(env.path("copy.png"), &path).unwrap();
    assert_eq!(
        env.art.image(image.clone()).await,
        Err(Unavailable::Unverified)
    );
    fs::remove_file(&path).unwrap();
    assert_eq!(
        env.art.image(image.clone()).await,
        Err(Unavailable::Missing)
    );
    let outside = crate::store::artwork::ImageRef {
        relative_path: "../copy.png".into(),
        ..image.clone()
    };
    assert_eq!(env.art.image(outside).await, Err(Unavailable::Unverified));
    let absolute = crate::store::artwork::ImageRef {
        relative_path: env.path("copy.png").to_string_lossy().into_owned(),
        ..image.clone()
    };
    assert_eq!(env.art.image(absolute).await, Err(Unavailable::Unverified));
    fs::create_dir(env.path("real")).unwrap();
    fs::write(env.path("real/x.png"), &good).unwrap();
    std::os::unix::fs::symlink(env.path("real"), env.path("linked")).unwrap();
    let through_link = crate::store::artwork::ImageRef {
        relative_path: "linked/x.png".into(),
        ..image.clone()
    };
    assert_eq!(
        env.art.image(through_link).await,
        Err(Unavailable::Unverified)
    );

    // The reference stays, nothing is searched or fetched to replace it, and
    // putting the right file back makes it available again.
    assert_eq!(env.selection("A").await, s);
    assert_eq!(env.drain().await, []);
    fs::write(&path, &good).unwrap();
    assert_eq!(env.art.image(image).await, Ok(good));
}

#[tokio::test]
async fn a_file_checked_before_is_hashed_again_only_when_it_changed() {
    let env = Env::new(&["A"]).await;
    let id = env.id("A").await;
    let v = env.selection("A").await.version;
    let s = env.art.upload(&id, v, samples::png(), None).await.unwrap();
    let image = image_of(&s).clone();
    let path = env.path(&image.relative_path);
    let good = fs::read(&path).unwrap();

    let hashed = env.art.verified.hashed();
    assert_eq!(env.art.image(image.clone()).await, Ok(good.clone()));
    assert_eq!(env.art.verified.hashed(), hashed + 1);
    // Served and told again: nothing is hashed while the file is as it was.
    for _ in 0..3 {
        assert_eq!(env.art.image(image.clone()).await, Ok(good.clone()));
        assert_eq!(env.art.image_state(image.clone()).await, Ok(()));
    }
    assert_eq!(env.art.verified.hashed(), hashed + 1);

    // Other bytes of the same size in the same file: found at the next check.
    let mut other = good.clone();
    let last = other.len() - 1;
    other[last] ^= 0xff;
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, &other))
        .unwrap();
    assert_eq!(
        env.art.image_state(image.clone()).await,
        Err(Unavailable::Mismatch)
    );
    assert_eq!(
        env.art.image(image.clone()).await,
        Err(Unavailable::Mismatch)
    );
    // The right bytes back (another file in its place): hashed once more.
    let hashed = env.art.verified.hashed();
    let back = env.path("back.png");
    fs::write(&back, &good).unwrap();
    fs::rename(&back, &path).unwrap();
    assert_eq!(env.art.image_state(image.clone()).await, Ok(()));
    assert_eq!(env.art.image(image).await, Ok(good));
    assert_eq!(env.art.verified.hashed(), hashed + 1);
}

#[tokio::test]
async fn image_files_are_read_a_few_at_a_time() {
    let env = Env::new(&["A"]).await;
    let id = env.id("A").await;
    let v = env.selection("A").await.version;
    let s = env.art.upload(&id, v, samples::png(), None).await.unwrap();
    let image = image_of(&s).clone();

    let taken = env
        .art
        .serving
        .clone()
        .acquire_many_owned(super::SERVING_SLOTS as u32)
        .await
        .unwrap();
    let art = env.art.clone();
    let served = tokio::spawn({
        let image = image.clone();
        async move { art.image(image).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!served.is_finished(), "a read waits for a turn");
    drop(taken);
    assert!(served.await.unwrap().is_ok());
}

#[tokio::test]
async fn the_cleanup_keeps_files_other_references_lead_to() {
    let env = Env::new(&["A", "B", "C", "D", "E"]).await;
    let (a, b, c, d, e) = (
        env.id("A").await,
        env.id("B").await,
        env.id("C").await,
        env.id("D").await,
        env.id("E").await,
    );
    let va = env.selection("A").await.version;
    let sa = env.art.upload(&a, va, samples::png(), None).await.unwrap();
    let path_a = image_of(&sa).relative_path.clone();
    // B is a copy of A with another image ID and the same file; C names the
    // same file another way. (A YAML import will make such references.)
    let copy = |work: String, path: String| {
        let image = image_of(&sa).clone();
        env.db.run::<_, DbError, _>(move |conn| {
            Ok(conn
                .execute(
                    "UPDATE work_artwork SET mode = 'manual', source = 'upload',
                         anilist_media_id = NULL, image_id = ?2, image_origin = 'upload',
                         image_path = ?3, image_size = ?4, image_sha256 = ?5,
                         image_format = 'png', job = NULL, version = version + 1
                     WHERE work_id = ?1",
                    rusqlite::params![
                        work,
                        crate::store::artwork::new_id(),
                        path,
                        image.byte_size as i64,
                        image.sha256
                    ],
                )
                .map(|_| ())?)
        })
    };
    copy(b.clone(), path_a.clone()).await.unwrap();
    copy(
        c.clone(),
        format!("artwork/./{}", &path_a["artwork/".len()..]),
    )
    .await
    .unwrap();

    let clear = |work: String| {
        let art = env.art.clone();
        async move {
            let v = art.store.selection(&work).await.unwrap().version;
            art.change(&work, v, UserChange::Clear).await.unwrap();
        }
    };
    clear(a.clone()).await;
    assert!(env.path(&path_a).exists(), "B still refers to it");
    clear(b.clone()).await;
    assert!(
        env.path(&path_a).exists(),
        "C refers to it by another spelling"
    );
    clear(c.clone()).await;
    assert!(!env.path(&path_a).exists());

    // A hard link is the same file.
    let vd = env.selection("D").await.version;
    let sd = env.art.upload(&d, vd, samples::jpeg(), None).await.unwrap();
    let path_d = image_of(&sd).relative_path.clone();
    fs::hard_link(env.path(&path_d), env.path("artwork/alias.jpg")).unwrap();
    {
        let image = image_of(&sd).clone();
        let e = e.clone();
        env.db
            .run::<_, DbError, _>(move |conn| {
                Ok(conn
                    .execute(
                        "UPDATE work_artwork SET mode = 'manual', source = 'upload',
                             image_id = 'e-image', image_origin = 'upload',
                             image_path = 'artwork/alias.jpg', image_size = ?2,
                             image_sha256 = ?3, image_format = 'jpeg', job = NULL
                         WHERE work_id = ?1",
                        rusqlite::params![e, image.byte_size as i64, image.sha256],
                    )
                    .map(|_| ())?)
            })
            .await
            .unwrap();
    }
    clear(d.clone()).await;
    assert!(env.path(&path_d).exists(), "E's alias is the same file");

    // A referenced place whose identity cannot be read: nothing is removed.
    fs::create_dir(env.path("locked")).unwrap();
    env.sql(
        "UPDATE work_artwork SET image_path = 'locked/x.png' WHERE work_id = ?1",
        e.clone(),
    )
    .await;
    fs::set_permissions(env.path("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let unreadable = fs::metadata(env.path("locked/x.png"))
        .is_err_and(|e| e.kind() != std::io::ErrorKind::NotFound);
    let cleaned = files::cleanup(&AppData::new(env.dir.path()), &env.art.store)
        .await
        .unwrap();
    fs::set_permissions(env.path("locked"), fs::Permissions::from_mode(0o755)).unwrap();
    if unreadable {
        // (Running as root reads it anyway; then the check below is moot.)
        assert!(cleaned.unsure);
        assert!(env.path(&path_d).exists());
    }
    // Once readable and no longer a reference, the file goes; the alias and a
    // stranger never do.
    fs::write(env.path("artwork/stranger.png"), b"?").unwrap();
    env.art.tidy().await;
    assert!(!env.path(&path_d).exists());
    assert!(env.path("artwork/alias.jpg").exists());
    assert!(env.path("artwork/stranger.png").exists());
}

// --- missing images ------------------------------------------------------------------------

#[tokio::test]
async fn a_missing_image_is_not_replaced_and_a_repair_fetches_the_same_entry() {
    let env = Env::new(&["Manual", "Auto"]).await;
    env.fake.add_search(
        "Auto",
        vec![env.fake.entry(5, "Auto", &[])],
        &samples::jpeg(),
    );
    env.fake.add_search(
        "Manual",
        vec![env.fake.entry(6, "Manual", &[])],
        &samples::png(),
    );
    env.drain().await;

    let m = env.id("Manual").await;
    let v = env.selection("Manual").await.version;
    // The user picks another entry than the automatic one.
    env.fake
        .state
        .lock()
        .unwrap()
        .media
        .insert(7, env.fake.entry(7, "Manual (2020)", &[]));
    env.fake
        .state
        .lock()
        .unwrap()
        .images
        .insert("7.jpg".into(), samples::webp());
    let manual = env.art.pick(&m, v, 7).await.unwrap();
    let auto = env.selection("Auto").await;

    for s in [&manual, &auto] {
        fs::remove_file(env.path(&image_of(s).relative_path)).unwrap();
    }
    let searches_before = env.fake.api_requests().len();

    // A restart, a rescan and reading the state ask for nothing.
    let restarted = Artwork::new(
        Db::open(env.dir.path().join("trss.db")).await.unwrap(),
        Some(AppData::new(env.dir.path())),
        env.fake.config(),
    )
    .with_spacing(Duration::ZERO);
    env.library
        .record_scan(&env.folder, Ok(scan(&["Manual", "Auto"])), 500)
        .await
        .unwrap();
    for name in ["Manual", "Auto"] {
        let s = restarted
            .store
            .selection(&env.id(name).await)
            .await
            .unwrap();
        assert_eq!(
            restarted.image(image_of(&s).clone()).await,
            Err(Unavailable::Missing)
        );
        assert!(s.job.is_none());
    }
    assert_eq!(restarted.run_next().await, None);
    assert_eq!(env.fake.api_requests().len(), searches_before);
    assert_eq!(env.selection("Manual").await, manual);

    // A repair asks AniList for the selected entry only, not a new search.
    for (name, s) in [("Manual", &manual), ("Auto", &auto)] {
        restarted
            .change(&env.id(name).await, s.version, UserChange::Repair)
            .await
            .unwrap();
    }
    assert_eq!(restarted.drain_for_test().await, 2);
    let asked: Vec<serde_json::Value> = env.fake.api_requests()[searches_before..]
        .iter()
        .map(|(_, v)| v.clone())
        .collect();
    assert!(asked.iter().all(|v| v.get("search").is_none()), "{asked:?}");
    assert!(asked.contains(&json!({ "id": 7 })));
    assert!(asked.contains(&json!({ "id": 5 })));
    let m = env.selection("Manual").await;
    assert_eq!((m.mode, m.anilist_media_id), (Mode::Manual, Some(7)));
    assert!(env.art.image(image_of(&m).clone()).await.is_ok());
    let a = env.selection("Auto").await;
    assert_eq!((a.mode, a.anilist_media_id), (Mode::Auto, Some(5)));
    assert!(env.art.image(image_of(&a).clone()).await.is_ok());
}

impl Artwork {
    /// Runs due jobs until none is left; how many ran.
    async fn drain_for_test(&self) -> usize {
        let mut n = 0;
        while self.run_next().await.is_some() {
            n += 1;
        }
        n
    }
}

/// An image that takes a while to decode in a test build.
fn slow_image() -> bytes::Bytes {
    samples::encoded(1600, 1600, ::image::ImageFormat::Png).into()
}

#[tokio::test]
async fn a_decode_keeps_its_turn_after_its_caller_stops_waiting() {
    let env = Env::new(&["A"]).await;
    let bytes = slow_image();
    // The caller gives up while the decode runs on its blocking thread.
    let gave_up = tokio::time::timeout(Duration::from_millis(5), env.art.verify(bytes)).await;
    assert!(gave_up.is_err(), "the decode should take longer than 5 ms");
    // The decode still runs, so no other decode may start beside it.
    assert_eq!(env.art.decoding.available_permits(), 0);
    let deadline = Instant::now() + Duration::from_secs(30);
    while env.art.decoding.available_permits() == 0 {
        assert!(Instant::now() < deadline, "the decode never ended");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn an_image_whose_caller_went_away_is_stored_whole_and_then_cleaned_up() {
    let env = Env::new(&["A"]).await;
    let stored = tokio::time::timeout(
        Duration::from_millis(5),
        env.art.store_image(slow_image(), Source::Upload, None),
    )
    .await;
    assert!(stored.is_err(), "storing should take longer than 5 ms");
    // The storing goes on to a published file whose identity is recorded,
    // never a staged file nobody can claim.
    let deadline = Instant::now() + Duration::from_secs(30);
    let row = loop {
        let rows = env
            .art
            .store
            .run(|c| Ok(crate::store::artwork::files_of_state(c, "staging")?))
            .await
            .unwrap();
        if let Some(row) = rows.into_iter().find(|r| r.dev.is_some()) {
            if env.files().len() == 1 {
                break row;
            }
        }
        assert!(Instant::now() < deadline, "the image was never stored");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(env.staging().is_empty());
    // Nothing took it: the recovery hands it to the cleanup.
    let late = row.created_at + STALE_STAGING.as_millis() as i64 + 1;
    let app = AppData::new(env.dir.path());
    assert_eq!(files::recover(&app, &env.art.store, late).await.unwrap(), 1);
    env.art.tidy().await;
    assert!(env.files().is_empty());
}

#[tokio::test]
async fn an_unregistered_works_cover_file_is_kept_and_served_again_when_it_comes_back() {
    let env = Env::new(&["A"]).await;
    let id = env.id("A").await;
    let v = env.selection("A").await.version;
    let uploaded = env.art.upload(&id, v, samples::png(), None).await.unwrap();
    let files = env.files();
    assert_eq!(files.len(), 1);

    env.library.remove_folder(&env.folder, 200).await.unwrap();
    // The cleanup counts the unregistered work's reference as live.
    env.art.maintain().await;
    assert_eq!(env.files(), files);
    // Its queue has nothing for it either way.
    assert_eq!(env.art.run_next().await, None);

    env.library
        .add_folder("/w".into(), scan(&["A"]), 300, &[])
        .await
        .unwrap();
    assert_eq!(env.id("A").await, id);
    let back = env.art.store.selection(&id).await.unwrap();
    assert_eq!(back, uploaded);
    assert_eq!(
        env.art.image(back.image.unwrap()).await.unwrap(),
        samples::png()
    );
    assert_eq!(env.art.run_next().await, None);
}
