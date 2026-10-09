//! The season flow end to end against a fake AniList ([`trss_anilist::fake`])
//! and a clock the test moves: the rows of ticket 0017 that are not about the
//! screen or the API.

use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use serde_json::{json, Value};

use crate::{
    artwork::Artwork,
    discovery::{Scan, ScannedWork, WorkRead},
    seasons::{queue::Ran, *},
    store::{
        library::LibraryStore,
        seasons::{Note, Origin},
    },
};
use trss_anilist::fake::Fake;

const HOUR: i64 = 60 * 60 * 1000;
const DAY: i64 = 24 * HOUR;

struct Env {
    _dir: tempfile::TempDir,
    db: Db,
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

/// An entry as AniList answers a search and the full question.
fn media(id: i64, title: &str, status: &str, episodes: Option<i64>) -> Value {
    json!({
        "id": id,
        "title": { "romaji": title, "english": null, "native": format!("{title} (원제)") },
        "synonyms": [],
        "format": "TV",
        "status": status,
        "episodes": episodes,
        "description": "One<br><br><i>Two</i> &amp; three",
        "startDate": { "year": 2022, "month": 7, "day": null },
        "endDate": { "year": 2022, "month": 9, "day": null },
        "genres": ["Action"],
        "studios": { "nodes": [{ "name": format!("Studio {id}"), "isAnimationStudio": true }] },
        "relations": { "edges": [] },
        "airingSchedule": { "nodes": [] },
    })
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
            None,
            fake.config(),
            Arc::new(move || reading.load(Ordering::SeqCst)),
        )
        .with_spacing(Duration::ZERO);
        let seasons = Seasons::over(db.clone(), &art);
        let library = LibraryStore::new(db.clone());
        let (folder, _) = library
            .add_folder("/w".into(), scan(works), 100, &[])
            .await
            .unwrap();
        Env {
            _dir: dir,
            db,
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

    fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    /// Sets the search for `text` to the single page `entries` and answers each by ID.
    fn serve(&self, text: &str, entries: Vec<Value>) {
        self.fake.add_search(text, entries, b"");
    }

    /// Answers the entry's ID with `entry` from now on.
    fn answer(&self, entry: Value) {
        let id = entry["id"].as_i64().unwrap();
        self.fake.state.lock().unwrap().media.insert(id, entry);
    }

    /// Runs due jobs until none is left.
    async fn drain(&self) -> Vec<Ran> {
        let mut ran = Vec::new();
        while let Some(r) = self.seasons.run_next().await {
            ran.push(r);
        }
        ran
    }

    fn requests(&self) -> usize {
        self.fake.api_requests().len()
    }
}

/// Whether `signal` was rung since it was last waited for.
async fn rung(signal: &tokio::sync::Notify) -> bool {
    tokio::time::timeout(Duration::from_millis(50), signal.notified())
        .await
        .is_ok()
}

fn ids(link: &crate::store::seasons::SeasonLink) -> Vec<i64> {
    link.entries.iter().map(|e| e.id).collect()
}

#[tokio::test]
async fn one_exact_title_links_the_first_season_as_auto_with_its_values() {
    let env = Env::new(&[("Lycoris Recoil", &[1, 2])]).await;
    let id = env.id("Lycoris Recoil").await;
    env.serve(
        "Lycoris Recoil",
        vec![
            media(1, "Lycoris Recoil", "FINISHED", Some(13)),
            media(2, "Lycoris Recoil: Other Name", "FINISHED", Some(1)),
        ],
    );
    let stored = env.seasons.stored();
    assert!(!rung(&stored).await);

    assert_eq!(env.drain().await, [Ran::Linked(1)]);
    // The worker hears that a season's entry is stored.
    assert!(rung(&stored).await);
    let link = env.seasons.store.link(&id, 1).await.unwrap();
    assert_eq!(ids(&link), [1]);
    assert_eq!(
        (link.origin, link.job, link.note),
        (Origin::Auto, None, None)
    );
    assert_eq!(link.version, 2);
    let entry = &link.entries[0];
    assert_eq!(entry.episodes, Some(13));
    assert_eq!(entry.studios, ["Studio 1"]);
    assert_eq!(entry.genres, ["Action"]);
    assert_eq!(entry.status.as_deref(), Some("FINISHED"));
    // The search and the entry: two requests; the other season has no job and nothing linked.
    assert_eq!(env.requests(), 2);
    let second = env.seasons.store.link(&id, 2).await.unwrap();
    assert_eq!(
        (second.version, second.entries.len(), second.job),
        (0, 0, None)
    );
    // Nothing to do afterwards: a restart or a rescan does not search again.
    assert!(env.drain().await.is_empty());
    env.library
        .record_scan(&env.folder, Ok(scan(&[("Lycoris Recoil", &[1, 2])])), 500)
        .await
        .unwrap();
    assert!(env.drain().await.is_empty());
}

#[tokio::test]
async fn the_title_is_compared_the_way_the_cover_does() {
    // Case, spaces and NFC only: the folder name is an exact title after those.
    let env = Env::new(&[("  lycoris   RECOIL ", &[1]), ("Lycoris Recoil 2", &[1])]).await;
    env.serve(
        "  lycoris   RECOIL ",
        vec![media(1, "Lycoris Recoil", "FINISHED", Some(13))],
    );
    // Only similar: a different number is a different title.
    env.serve(
        "Lycoris Recoil 2",
        vec![media(1, "Lycoris Recoil", "FINISHED", Some(13))],
    );
    let ran = env.drain().await;
    assert_eq!(ran.len(), 2);
    assert!(ran.contains(&Ran::Linked(1)));
    assert!(ran.contains(&Ran::Left(Note::NoMatch)));
}

#[tokio::test]
async fn a_second_candidate_with_the_same_title_links_nothing_and_a_note_says_why() {
    let env = Env::new(&[("Clevatess", &[1])]).await;
    let id = env.id("Clevatess").await;
    env.serve(
        "Clevatess",
        vec![
            media(1, "Clevatess", "FINISHED", Some(12)),
            media(2, "Clevatess", "RELEASING", None),
        ],
    );
    assert_eq!(env.drain().await, [Ran::Left(Note::Ambiguous)]);
    let link = env.seasons.store.link(&id, 1).await.unwrap();
    assert_eq!(
        (link.entries.len(), link.note, link.job),
        (0, Some(Note::Ambiguous), None)
    );
    // The user can still search and choose.
    let chosen = env
        .seasons
        .set_links(&id, 1, link.version, vec![2])
        .await
        .unwrap();
    assert_eq!(
        (ids(&chosen), chosen.origin, chosen.note),
        (vec![2], Origin::User, None)
    );
}

#[tokio::test]
async fn a_search_that_was_not_read_to_its_end_links_nothing() {
    let env = Env::new(&[("Common", &[1])]).await;
    // Five pages: more than the automatic search reads.
    let pages: Vec<Vec<Value>> = (1..=5)
        .map(|p| vec![media(p, &format!("Other {p}"), "FINISHED", Some(12))])
        .collect();
    {
        let mut state = env.fake.state.lock().unwrap();
        for page in &pages {
            for e in page {
                state.media.insert(e["id"].as_i64().unwrap(), e.clone());
            }
        }
        state.searches.insert("Common".to_owned(), pages);
    }
    assert_eq!(env.drain().await, [Ran::Left(Note::Incomplete)]);
}

#[tokio::test]
async fn two_parts_in_order_are_one_season() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.fake.state.lock().unwrap().searches.clear();
    // The automatic search finds nothing; the user links Part 1 then Part 2.
    assert_eq!(env.drain().await, [Ran::Left(Note::NoMatch)]);
    let mut part1 = media(11, "Show Part 1", "FINISHED", Some(12));
    part1["startDate"] = json!({ "year": 2022, "month": 4, "day": null });
    part1["endDate"] = json!({ "year": 2022, "month": 6, "day": null });
    let part2 = media(12, "Show Part 2", "FINISHED", Some(13));
    env.answer(part1);
    env.answer(part2);
    let version = env.seasons.store.link(&id, 1).await.unwrap().version;
    let link = env
        .seasons
        .set_links(&id, 1, version, vec![11, 12])
        .await
        .unwrap();
    assert_eq!(ids(&link), [11, 12]);
    let combined = combine::combine(&link.entries).unwrap();
    assert_eq!(combined.episodes, Some(25));
    assert_eq!(combined.start.month, Some(4));
    assert_eq!(combined.end.month, Some(9));
    assert_eq!(combined.studios, ["Studio 11", "Studio 12"]);
    // An entry stored already is not asked for again; one more link asks for the new entry only.
    let before = env.requests();
    env.seasons
        .set_links(&id, 1, link.version, vec![12, 11])
        .await
        .unwrap();
    assert_eq!(env.requests(), before);
}

#[tokio::test]
async fn a_stale_version_changes_nothing_and_a_missing_entry_links_nothing() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.drain().await;
    env.answer(media(1, "A", "FINISHED", Some(12)));
    let v = env.seasons.store.link(&id, 1).await.unwrap().version;
    env.seasons.set_links(&id, 1, v, vec![1]).await.unwrap();

    // An old version is refused before AniList is asked about anything.
    let before = env.requests();
    match env.seasons.set_links(&id, 1, v, vec![999]).await {
        Err(ActionError::Store(SeasonError::Conflict(current))) => {
            assert_eq!((current.version, ids(&current)), (v + 1, vec![1]));
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(env.requests(), before);
    // An ID AniList does not have links nothing and changes nothing.
    match env.seasons.set_links(&id, 1, v + 1, vec![999]).await {
        Err(ActionError::NoEntry(999)) => {}
        other => panic!("expected no entry, got {other:?}"),
    }
    let link = env.seasons.store.link(&id, 1).await.unwrap();
    assert_eq!((link.version, ids(&link)), (v + 1, vec![1]));
    assert!(matches!(
        env.seasons.set_links(&id, 7, 0, vec![1]).await,
        Err(ActionError::Store(SeasonError::NotFound))
    ));
}

#[tokio::test]
async fn a_season_links_eight_entries_at_most_and_the_ninth_is_refused_before_any_request() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.drain().await;
    for n in 1..=9 {
        env.answer(media(n, &format!("Part {n}"), "FINISHED", Some(12)));
    }
    let v = env.seasons.store.link(&id, 1).await.unwrap().version;
    assert_eq!(MAX_ENTRIES, 8);

    let before = env.requests();
    match env.seasons.set_links(&id, 1, v, (1..=9).collect()).await {
        Err(ActionError::Store(SeasonError::Invalid(_))) => {}
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(env.requests(), before, "nothing was asked of AniList");
    let link = env.seasons.store.link(&id, 1).await.unwrap();
    assert_eq!((link.version, ids(&link)), (v, vec![]));

    // That many are taken, in the order given.
    let link = env
        .seasons
        .set_links(&id, 1, v, (1..=8).rev().collect())
        .await
        .unwrap();
    assert_eq!(ids(&link), [8, 7, 6, 5, 4, 3, 2, 1]);
}

#[tokio::test]
async fn an_automatic_result_that_finishes_after_the_user_chose_changes_nothing() {
    let env = Env::new(&[("Lycoris Recoil", &[1])]).await;
    let id = env.id("Lycoris Recoil").await;
    env.serve(
        "Lycoris Recoil",
        vec![media(1, "Lycoris Recoil", "FINISHED", Some(13))],
    );
    env.answer(media(2, "Chosen", "FINISHED", Some(12)));

    // The worker takes the job at version 1...
    let job = env
        .seasons
        .store
        .next_search(env.seasons.now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.version, 1);
    // ...the user chooses another entry meanwhile...
    let chosen = env.seasons.set_links(&id, 1, 1, vec![2]).await.unwrap();
    // ...and the search ends, clear match and all.
    assert_eq!(env.seasons.run_search(&job).await, Ran::Dropped);
    let link = env.seasons.store.link(&id, 1).await.unwrap();
    assert_eq!(
        (link.version, link.origin, ids(&link)),
        (chosen.version, Origin::User, vec![2])
    );

    // The same holds for a search that finds nothing, and for one the user unlinked after.
    env.seasons
        .set_links(&id, 1, chosen.version, vec![])
        .await
        .unwrap();
    assert!(env.drain().await.is_empty());
}

#[tokio::test]
async fn the_next_season_is_offered_the_sequels_and_linked_only_when_the_user_confirms() {
    let env = Env::new(&[("Lycoris Recoil", &[1, 2])]).await;
    let id = env.id("Lycoris Recoil").await;
    let mut first = media(1, "Lycoris Recoil", "FINISHED", Some(13));
    first["relations"] = json!({ "edges": [
        { "relationType": "SEQUEL", "node": { "id": 20, "type": "ANIME", "format": "TV",
            "status": "NOT_YET_RELEASED", "title": { "romaji": "Season 2", "english": null, "native": null },
            "startDate": { "year": 2026, "month": 4, "day": null } } },
        { "relationType": "SEQUEL", "node": { "id": 21, "type": "ANIME", "format": "MOVIE",
            "status": "FINISHED", "title": { "romaji": "The Movie", "english": null, "native": null },
            "startDate": { "year": 2025, "month": 1, "day": 1 } } },
        { "relationType": "ADAPTATION", "node": { "id": 99, "type": "MANGA" } }
    ] });
    env.serve("Lycoris Recoil", vec![first]);
    assert_eq!(env.drain().await, [Ran::Linked(1)]);

    let first_link = env.seasons.store.link(&id, 1).await.unwrap();
    let offered = combine::suggestions(Some(&first_link.entries), &[]);
    assert_eq!(offered.iter().map(|s| s.id).collect::<Vec<_>>(), [20, 21]);
    assert_eq!(offered[1].format.as_deref(), Some("MOVIE"));

    // Nothing is linked and nothing is waiting: no job, no automatic result, no change.
    let second = env.seasons.store.link(&id, 2).await.unwrap();
    assert_eq!(
        (second.version, second.entries.len(), second.job),
        (0, 0, None)
    );
    assert!(env.drain().await.is_empty());
    assert_eq!(
        env.seasons.store.link(&id, 2).await.unwrap().entries.len(),
        0
    );

    // The user confirms one.
    env.answer(media(20, "Season 2", "NOT_YET_RELEASED", Some(12)));
    let confirmed = env.seasons.set_links(&id, 2, 0, vec![20]).await.unwrap();
    assert_eq!(
        (ids(&confirmed), confirmed.origin),
        (vec![20], Origin::User)
    );
    assert!(combine::suggestions(Some(&first_link.entries), &confirmed.entries).is_empty());
}

#[tokio::test]
async fn entries_that_are_not_finished_are_received_again_a_day_later_and_finished_ones_are_not() {
    let env = Env::new(&[("Show", &[1]), ("Other", &[1])]).await;
    let show = env.id("Show").await;
    env.drain().await;
    env.answer(media(1, "Airing", "RELEASING", None));
    env.answer(media(2, "Upcoming", "NOT_YET_RELEASED", None));
    env.answer(media(3, "Done", "FINISHED", Some(12)));
    let v = env.seasons.store.link(&show, 1).await.unwrap().version;
    env.seasons
        .set_links(&show, 1, v, vec![1, 2, 3])
        .await
        .unwrap();
    let received = env.requests();
    let stored = env.seasons.stored();
    rung(&stored).await;

    // Just received: nothing is due, however often the queue looks.
    assert!(env.drain().await.is_empty());
    env.advance(DAY - 1);
    assert!(env.drain().await.is_empty());
    assert_eq!(env.requests(), received);
    assert!(!rung(&stored).await);

    // A day on, AniList says more: the two active entries are received again, the finished one not.
    env.answer(media(1, "Airing", "RELEASING", Some(24)));
    env.answer(media(2, "Upcoming", "RELEASING", Some(12)));
    env.answer(media(3, "Done", "FINISHED", Some(99)));
    env.advance(1);
    let mut ran = env.drain().await;
    ran.sort_by_key(|r| format!("{r:?}"));
    assert_eq!(ran, [Ran::Refreshed(1), Ran::Refreshed(2)]);
    assert!(rung(&stored).await, "the worker hears of the new counts");
    assert_eq!(env.requests(), received + 2);
    let link = env.seasons.store.link(&show, 1).await.unwrap();
    assert_eq!(
        link.entries
            .iter()
            .map(|e| (e.id, e.episodes))
            .collect::<Vec<_>>(),
        [(1, Some(24)), (2, Some(12)), (3, Some(12))]
    );
    assert_eq!(link.version, v + 1, "a refresh is no change of the link");

    // Both are airing now: due again a day after they were received, not before.
    env.advance(DAY - 1);
    assert!(env.drain().await.is_empty());
    env.answer(media(1, "Airing", "FINISHED", Some(24)));
    env.advance(1);
    assert_eq!(env.drain().await, [Ran::Refreshed(1), Ran::Refreshed(2)]);
    // Entry 1 finished: it is not asked for again, 2 still is.
    env.advance(DAY * 3);
    assert_eq!(env.drain().await, [Ran::Refreshed(2)]);
    assert_eq!(
        env.seasons.store.entry(3).await.unwrap().unwrap().episodes,
        Some(12)
    );
}

#[tokio::test]
async fn an_entry_with_a_long_schedule_is_stored_with_every_airing() {
    let env = Env::new(&[("Show", &[1])]).await;
    let show = env.id("Show").await;
    env.drain().await;
    // AniList answers 25 airings to a page; the entry has 60, and is finished.
    let mut long = media(1, "Long", "FINISHED", Some(60));
    long["airingSchedule"] = json!({
        "nodes": (1..=60)
            .map(|i| json!({ "episode": i, "airingAt": 1_000 * i }))
            .collect::<Vec<_>>()
    });
    env.answer(long);
    let v = env.seasons.store.link(&show, 1).await.unwrap().version;
    env.seasons.set_links(&show, 1, v, vec![1]).await.unwrap();

    let entry = env.seasons.store.entry(1).await.unwrap().unwrap();
    assert_eq!(entry.airing.len(), 60);
    assert_eq!(entry.airing[59].episode, 60);
    assert_eq!(entry.airing[59].at, 60_000);
    // The entry, then pages 2 and 3.
    let requests = env.fake.api_requests();
    let pages: Vec<Option<u64>> = requests.iter().map(|(_, v)| v["page"].as_u64()).collect();
    assert_eq!(&pages[pages.len() - 3..], [None, Some(2), Some(3)]);
}

#[tokio::test]
async fn the_user_can_receive_a_finished_entry_again() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.drain().await;
    env.answer(media(3, "Done", "FINISHED", Some(12)));
    let v = env.seasons.store.link(&id, 1).await.unwrap().version;
    env.seasons.set_links(&id, 1, v, vec![3]).await.unwrap();
    env.advance(30 * DAY);
    assert!(env.drain().await.is_empty());

    let mut changed = media(3, "Done", "FINISHED", Some(12));
    changed["description"] = json!("Fixed text");
    env.answer(changed);
    let link = env.seasons.refresh_season(&id, 1).await.unwrap();
    assert_eq!(link.entries[0].description.as_deref(), Some("Fixed text"));
    assert_eq!(link.entries[0].fetched_at, env.seasons.now());
    assert_eq!(link.version, v + 1);
}

#[tokio::test]
async fn refresh_requests_keep_the_pace_the_covers_use() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.drain().await;
    for n in 1..=3 {
        env.answer(media(n, &format!("Airing {n}"), "RELEASING", None));
    }
    let v = env.seasons.store.link(&id, 1).await.unwrap().version;
    env.seasons
        .set_links(&id, 1, v, vec![1, 2, 3])
        .await
        .unwrap();
    env.advance(DAY);

    // 120 ms between two requests, shared with the cover queue through the database.
    let spaced = env.art.clone().with_spacing(Duration::from_millis(120));
    let seasons = Seasons::over(env.db.clone(), &spaced);
    let started = Instant::now();
    let first_before = env.fake.api_requests().len();
    for _ in 0..3 {
        assert!(matches!(seasons.run_next().await, Some(Ran::Refreshed(_))));
    }
    let requests = env.fake.api_requests();
    let times: Vec<Instant> = requests[first_before..].iter().map(|(t, _)| *t).collect();
    assert_eq!(times.len(), 3);
    for pair in times.windows(2) {
        assert!(
            pair[1].duration_since(pair[0]) >= Duration::from_millis(100),
            "{pair:?}"
        );
    }
    assert!(started.elapsed() >= Duration::from_millis(220));
}

#[tokio::test]
async fn a_429_holds_the_refresh_for_as_long_as_it_says_and_a_failure_waits_an_hour() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.drain().await;
    env.answer(media(1, "Airing", "RELEASING", None));
    let v = env.seasons.store.link(&id, 1).await.unwrap().version;
    env.seasons.set_links(&id, 1, v, vec![1]).await.unwrap();
    env.advance(DAY);

    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = 1800;
    }
    let requests = env.requests();
    assert_eq!(env.seasons.run_next().await, Some(Ran::RefreshLater(1)));
    assert_eq!(env.requests(), requests + 1);
    // Nothing is sent while AniList asked to wait.
    env.advance(1800 * 1000 - 1);
    assert!(env.drain().await.is_empty());
    assert_eq!(env.requests(), requests + 1);
    env.advance(1);
    assert_eq!(env.drain().await, [Ran::Refreshed(1)]);

    // A server error: the entry waits an hour before the next try.
    env.advance(DAY);
    env.fake.state.lock().unwrap().failing = 1;
    assert_eq!(env.seasons.run_next().await, Some(Ran::RefreshLater(1)));
    env.advance(HOUR - 1);
    assert!(env.drain().await.is_empty());
    env.advance(1);
    assert_eq!(env.drain().await, [Ran::Refreshed(1)]);
}

#[tokio::test]
async fn an_entry_that_is_gone_keeps_what_was_stored_and_is_not_asked_for_every_cycle() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.drain().await;
    env.answer(media(1, "Airing", "RELEASING", Some(12)));
    let v = env.seasons.store.link(&id, 1).await.unwrap().version;
    env.seasons.set_links(&id, 1, v, vec![1]).await.unwrap();
    env.fake.state.lock().unwrap().media.clear();
    env.advance(DAY);
    assert_eq!(env.seasons.run_next().await, Some(Ran::RefreshLater(1)));
    assert!(env.drain().await.is_empty());
    let kept = env.seasons.store.entry(1).await.unwrap().unwrap();
    assert_eq!(kept.episodes, Some(12));
    assert_eq!(kept.fetched_at, env.seasons.now());
}

#[tokio::test]
async fn a_search_or_refresh_that_panics_is_put_off_like_a_failure_and_the_next_one_runs() {
    let env = Env::new(&[("Show", &[1]), ("Other", &[1])]).await;
    env.serve("Show", vec![media(1, "Show", "RELEASING", None)]);
    env.serve("Other", vec![media(2, "Other", "FINISHED", Some(12))]);
    // The search taken first panics.
    let first = env.seasons.store.next_search(env.seasons.now()).await;
    let first = first.unwrap().unwrap().work_id;
    let (panicked, next) = match first == env.id("Show").await {
        true => (1, 2),
        false => (2, 1),
    };
    let queue = crate::seasons::queue::QUEUE;
    trss_core::queue::testing::panic_next(queue, &format!("search for work {first} season 1"));
    assert_eq!(env.drain().await, [Ran::Later, Ran::Linked(next)]);
    let job = env
        .seasons
        .store
        .link(&first, 1)
        .await
        .unwrap()
        .job
        .unwrap();
    assert_eq!(job.attempts, 1);
    assert!((60_000..65_000).contains(&(job.not_before.unwrap() - env.seasons.now())));
    env.advance(65_000);
    assert_eq!(env.drain().await, [Ran::Linked(panicked)]);

    // A refresh that panics waits an hour, as a failed one does.
    env.advance(DAY);
    trss_core::queue::testing::panic_next(queue, "refresh of entry 1");
    let requests = env.requests();
    assert_eq!(env.seasons.run_next().await, Some(Ran::RefreshLater(1)));
    assert_eq!(env.requests(), requests);
    env.advance(HOUR - 1);
    assert!(env.drain().await.is_empty());
    env.advance(1);
    assert_eq!(env.drain().await, [Ran::Refreshed(1)]);
}

#[tokio::test]
async fn a_failed_search_is_tried_again_later_and_given_up_after_three_failures() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.fake.state.lock().unwrap().failing = 100;
    assert_eq!(env.seasons.run_next().await, Some(Ran::Later));
    let link = env.seasons.store.link(&id, 1).await.unwrap();
    let job = link.job.unwrap();
    assert_eq!(job.attempts, 1);
    assert!((60_000..65_000).contains(&(job.not_before.unwrap() - env.seasons.now())));
    // Not due yet.
    assert_eq!(env.seasons.run_next().await, None);
    env.advance(DAY);
    assert_eq!(env.seasons.run_next().await, Some(Ran::Later));
    env.advance(DAY);
    assert_eq!(env.seasons.run_next().await, Some(Ran::Later));
    env.advance(DAY);
    assert_eq!(env.seasons.run_next().await, Some(Ran::Left(Note::Failed)));
    let link = env.seasons.store.link(&id, 1).await.unwrap();
    assert_eq!(
        (link.job, link.note, link.version),
        (None, Some(Note::Failed), 1)
    );
    // The user asks again once AniList answers.
    env.fake.state.lock().unwrap().failing = 0;
    env.serve("Show", vec![media(1, "Show", "FINISHED", Some(12))]);
    let again = env.seasons.restart_auto(&id, 1, 1).await.unwrap();
    assert!(again.job.is_some());
    assert_eq!(env.drain().await, [Ran::Linked(1)]);
}

/// Makes every write that `sql` names fail, as a full disk or a broken file would.
async fn refuse(env: &Env, sql: &'static str) {
    env.db
        .run::<_, trss_core::DbError, _>(move |conn| Ok(conn.execute_batch(sql)?))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_search_whose_outcome_cannot_be_written_waits_like_a_failure_instead_of_asking_again_at_once(
) {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.serve("Show", vec![media(1, "Show", "FINISHED", Some(12))]);
    refuse(
        &env,
        "CREATE TRIGGER refuse BEFORE INSERT ON anilist_entries
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .await;
    assert_eq!(env.seasons.run_next().await, Some(Ran::Later));
    let job = env.seasons.store.link(&id, 1).await.unwrap().job.unwrap();
    assert_eq!(job.attempts, 1);
    assert!((60_000..65_000).contains(&(job.not_before.unwrap() - env.seasons.now())));
    let requests = env.requests();
    assert_eq!(env.seasons.run_next().await, None);
    assert_eq!(env.requests(), requests);

    refuse(&env, "DROP TRIGGER refuse;").await;
    env.advance(DAY);
    assert_eq!(env.drain().await, [Ran::Linked(1)]);
}

#[tokio::test]
async fn a_refresh_that_cannot_be_written_waits_an_hour() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.drain().await;
    env.answer(media(1, "Airing", "RELEASING", None));
    let v = env.seasons.store.link(&id, 1).await.unwrap().version;
    env.seasons.set_links(&id, 1, v, vec![1]).await.unwrap();
    refuse(
        &env,
        "CREATE TRIGGER refuse BEFORE UPDATE ON anilist_entries
         WHEN NEW.fetched_at <> OLD.fetched_at
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .await;
    env.advance(DAY);
    assert_eq!(env.seasons.run_next().await, Some(Ran::RefreshLater(1)));
    let requests = env.requests();
    assert_eq!(env.seasons.run_next().await, None);
    assert_eq!(env.requests(), requests);

    refuse(&env, "DROP TRIGGER refuse;").await;
    env.advance(HOUR - 1);
    assert!(env.drain().await.is_empty());
    env.advance(1);
    assert_eq!(env.drain().await, [Ran::Refreshed(1)]);
}
