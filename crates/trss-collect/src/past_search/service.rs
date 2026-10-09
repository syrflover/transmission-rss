//! The searches this process is running or has finished, which a screen polls.
//!
//! A search takes up to a minute (its extra pages are spaced, see
//! [`super::client`]), so it runs as a task and the screen asks how far it is.
//! The results stay in memory only, never in the database: a search leaves no
//! channel and no history. What is kept of a finished search is the preview and,
//! for each item the person may pick, the title and link history would store, so
//! that receiving an item names it by its key and the browser never supplies a
//! link.
//!
//! The registry is bounded. It keeps at most [`MAX_SEARCHES`] searches, drops one
//! after [`KEEP`], and a new search of a rule ends that rule's earlier one.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use tokio::task::AbortHandle;

use crate::{
    feed::FeedItem,
    past_search::{
        client::{wait_phrase, SearchClient, SearchError, REQUEST_SPACING},
        judge::{judge, Preview, Range, Result as Judged},
        run::{run, Limits},
        world,
    },
    plan::picks,
    revision::file_crc32,
    store::{
        channels::{Channel, Rule},
        history::HistoryItem,
        search_pace::SearchPace,
        status::TorrentListing,
    },
};
use trss_transmission::Redactor;

/// How many searches are kept at once.
pub const MAX_SEARCHES: usize = 6;
/// How long a search is kept after it started.
pub const KEEP: Duration = Duration::from_secs(30 * 60);

/// What a search needs, gathered when it is accepted.
pub struct Spec {
    pub channel: Channel,
    pub rule: Rule,
    pub query: String,
    pub range: Range,
    /// The rule's work folder and episode conversion.
    pub save_path: std::path::PathBuf,
    pub offset: i64,
    pub season: Option<u32>,
    /// The channel's items history says Transmission holds. Dropped once the
    /// search has built its picture of the work.
    pub settled: Vec<HistoryItem>,
    /// The torrents Transmission held when the worker last looked, if it has;
    /// they tell which of `settled` were removed since.
    pub listing: Option<TorrentListing>,
    /// The titles history holds for the channel. Dropped like `settled`.
    pub titles: Vec<String>,
    /// Whether the channel's history is longer than what `settled` and
    /// `titles` hold (the newest are held).
    pub history_cut: bool,
    /// Redacts the channel's secret values from what is reported.
    pub redactor: Redactor,
}

/// A finished search.
#[derive(Debug)]
pub struct Outcome {
    pub range: Range,
    pub query: String,
    pub preview: Preview,
    pub notes: Vec<String>,
    pub first_full: bool,
    pub extra_sent: usize,
    pub extra_needed: usize,
    /// For each item of the preview: the title and the link as history stores them.
    pub storable: HashMap<String, Stored>,
    pub rule_id: String,
}

/// An item as history would store it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub title: String,
    pub link: String,
    /// Whether history says Transmission took the item and it had gone from the
    /// work when the search looked ([`world::departed`]): it is received again
    /// though its result says `received`.
    pub departed: bool,
}

#[derive(Debug, Clone)]
pub enum Status {
    Running {
        /// The extra searches sent, and how many are planned (0 until known).
        sent: usize,
        needed: usize,
    },
    Done(Arc<Outcome>),
    Failed(String),
}

struct Entry {
    rule_id: String,
    started: Instant,
    status: Status,
    abort: Option<AbortHandle>,
}

#[derive(Default)]
struct Registry {
    searches: HashMap<String, Entry>,
}

impl Registry {
    /// Forgets the searches past [`KEEP`], and the oldest ones beyond
    /// [`MAX_SEARCHES`]; the task of each is aborted.
    fn sweep(&mut self) {
        let expired: Vec<String> = self
            .searches
            .iter()
            .filter(|(_, entry)| entry.started.elapsed() >= KEEP)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.remove(&id);
        }
        while self.searches.len() > MAX_SEARCHES {
            let oldest = self
                .searches
                .iter()
                .min_by_key(|(_, e)| e.started)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => self.remove(&id),
                None => break,
            }
        }
    }

    fn remove(&mut self, id: &str) {
        if let Some(entry) = self.searches.remove(id) {
            if let Some(abort) = entry.abort {
                abort.abort();
            }
        }
    }
}

/// Why a finished search has no item with a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolve {
    /// The search is not (any more) here: it ended, or this process restarted.
    Gone,
    /// The search belongs to another rule.
    OtherRule,
    /// The search has not finished.
    Running,
    /// The search has no such item.
    NoItem,
}

/// Cheap to clone; the clones share the registry.
#[derive(Clone)]
pub struct PastSearch {
    pace: SearchPace,
    spacing: Duration,
    registry: Arc<Mutex<Registry>>,
}

impl PastSearch {
    pub fn new(pace: SearchPace) -> Self {
        PastSearch {
            pace,
            spacing: REQUEST_SPACING,
            registry: Arc::default(),
        }
    }

    /// Overrides the time between requests (tests).
    pub fn with_spacing(mut self, spacing: Duration) -> Self {
        self.spacing = spacing;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Starts a search and returns its ID. The rule's earlier search ends.
    pub fn start(&self, spec: Spec) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let rule_id = spec.rule.id.clone();
        {
            let mut registry = self.lock();
            registry.searches.retain(|_, entry| {
                let same = entry.rule_id == rule_id;
                if same {
                    if let Some(abort) = &entry.abort {
                        abort.abort();
                    }
                }
                !same
            });
            registry.searches.insert(
                id.clone(),
                Entry {
                    rule_id: rule_id.clone(),
                    started: Instant::now(),
                    status: Status::Running { sent: 0, needed: 0 },
                    abort: None,
                },
            );
            registry.sweep();
        }

        let this = self.clone();
        let task_id = id.clone();
        let work = tokio::spawn({
            let this = this.clone();
            let id = id.clone();
            async move {
                let status = match this.search(&id, spec).await {
                    Ok(outcome) => Status::Done(Arc::new(outcome)),
                    Err(sentence) => Status::Failed(sentence),
                };
                this.set(&id, status);
            }
        });
        if let Some(entry) = self.lock().searches.get_mut(&id) {
            entry.abort = Some(work.abort_handle());
        }
        // A search that panicked is told as failed; one that was aborted has
        // left the registry already.
        tokio::spawn(async move {
            if let Err(err) = work.await {
                if err.is_panic() {
                    this.set(
                        &task_id,
                        Status::Failed("검색 중 오류가 나서 끝내지 못했어요.".to_owned()),
                    );
                }
            }
        });
        id
    }

    fn set(&self, id: &str, status: Status) {
        if let Some(entry) = self.lock().searches.get_mut(id) {
            entry.status = status;
            entry.abort = None;
        }
    }

    fn progress(&self, id: &str, sent: usize, needed: usize) {
        if let Some(entry) = self.lock().searches.get_mut(id) {
            if matches!(entry.status, Status::Running { .. }) {
                entry.status = Status::Running { sent, needed };
            }
        }
    }

    /// The state of a search, or `None` when it is gone.
    pub fn status(&self, id: &str) -> Option<(String, Status)> {
        let mut registry = self.lock();
        registry.sweep();
        registry
            .searches
            .get(id)
            .map(|entry| (entry.rule_id.clone(), entry.status.clone()))
    }

    /// The running search of a rule, if any.
    pub fn running_of(&self, rule_id: &str) -> Option<String> {
        self.lock()
            .searches
            .iter()
            .find(|(_, e)| e.rule_id == rule_id && matches!(e.status, Status::Running { .. }))
            .map(|(id, _)| id.clone())
    }

    /// Ends a search and forgets it. Returns whether it was here.
    pub fn cancel(&self, id: &str) -> bool {
        match self.lock().searches.remove(id) {
            Some(entry) => {
                if let Some(abort) = entry.abort {
                    abort.abort();
                }
                true
            }
            None => false,
        }
    }

    /// The item `key` of the finished search `id` of rule `rule_id`, as history
    /// stores it.
    pub fn resolve(&self, id: &str, rule_id: &str, key: &str) -> Result<Stored, Resolve> {
        let mut registry = self.lock();
        // A search is kept for [`KEEP`] and no longer, whether or not
        // anything polled it in the meantime.
        registry.sweep();
        let entry = registry.searches.get(id).ok_or(Resolve::Gone)?;
        if entry.rule_id != rule_id {
            return Err(Resolve::OtherRule);
        }
        match &entry.status {
            Status::Running { .. } => Err(Resolve::Running),
            Status::Failed(_) => Err(Resolve::NoItem),
            Status::Done(outcome) => outcome.storable.get(key).cloned().ok_or(Resolve::NoItem),
        }
    }

    async fn search(&self, id: &str, spec: Spec) -> Result<Outcome, String> {
        let Spec {
            channel,
            rule,
            query,
            range,
            save_path,
            offset,
            season,
            settled,
            listing,
            titles,
            history_cut,
            redactor,
        } = spec;

        // Reading the folder and going through the history are blocking work
        // on as much as a few thousand records; `settled` and `titles` are
        // dropped with the closure, not held for the minute a search can take.
        let world = {
            let rule_id = rule.id.clone();
            tokio::task::spawn_blocking(move || {
                let folder = world::read_folder(&save_path)?;
                Ok(world::build(
                    offset,
                    season,
                    folder,
                    &rule_id,
                    &settled,
                    listing.as_ref(),
                    &titles,
                ))
            })
            .await
            .map_err(|_| "작품 폴더를 읽는 중 오류가 났어요.".to_owned())?
            .map_err(|err: std::io::Error| format!("작품 폴더를 읽지 못했어요: {}", err.kind()))?
        };

        let judging: Arc<dyn Fn(&str) -> bool + Send + Sync> = {
            let (channel, rule) = (channel.clone(), rule.clone());
            Arc::new(move |title| picks(&channel, &rule, title))
        };
        let phrase = match (&rule.r#match, rule.regex) {
            (Some(phrase), false) => phrase.clone(),
            _ => String::new(),
        };
        let client = SearchClient::new(self.pace.clone())
            .map_err(|_| "검색 요청을 준비하지 못했어요.".to_owned())?
            .with_spacing(self.spacing);
        let progress_id = id.to_owned();
        let progress = {
            let this = self.clone();
            move |sent: usize, needed: usize| this.progress(&progress_id, sent, needed)
        };
        let found = run(
            &client,
            &channel,
            &redactor,
            &query,
            &phrase,
            range,
            &world,
            &*judging,
            Limits::default(),
            &progress,
        )
        .await
        .map_err(search_error)?;

        let results: Vec<Judged> = found
            .items
            .iter()
            .map(|item| Judged {
                key: item.identity_key.clone(),
                title: item.title.clone(),
                shown: item.stored_title.clone(),
            })
            .collect();
        let judged = judging.clone();
        let departed = world.departed.clone();
        let preview = tokio::task::spawn_blocking(move || {
            judge(&results, range, &world, &*judged, &mut |path| {
                file_crc32(path)
            })
        })
        .await
        .map_err(|_| "결과를 판정하는 중 오류가 났어요.".to_owned())?;

        let by_key: HashMap<&str, &FeedItem> = found
            .items
            .iter()
            .map(|item| (item.identity_key.as_str(), item))
            .collect();
        let storable = preview
            .items
            .iter()
            .filter_map(|listed| {
                by_key.get(listed.key.as_str()).map(|item| {
                    (
                        listed.key.clone(),
                        Stored {
                            title: item.stored_title.clone(),
                            link: item.stored_link.clone(),
                            departed: departed.contains(&listed.key),
                        },
                    )
                })
            })
            .collect();
        let mut notes = found.notes;
        if history_cut {
            notes.push(
                "채널의 기록이 많아서 최근 기록만 살폈어요. 오래전에 받은 항목은 받은 것으로 보이지 않을 수 있어요."
                    .to_owned(),
            );
        }
        Ok(Outcome {
            range,
            query,
            preview,
            notes,
            first_full: found.first_full,
            extra_sent: found.extra_sent,
            extra_needed: found.extra_needed,
            storable,
            rule_id: rule.id,
        })
    }
}

fn search_error(err: SearchError) -> String {
    match err {
        SearchError::BadAddress => {
            "채널 주소로는 검색 주소를 만들 수 없어요. 채널의 주소를 확인해 주세요.".to_owned()
        }
        SearchError::Pace(_) => {
            "검색 요청 간격을 확인하지 못했어요. 잠시 뒤 다시 시도해 주세요.".to_owned()
        }
        SearchError::Wait(wait) => format!(
            "검색 서버에 요청을 보내는 간격 제한이 걸려 있어서 지금은 검색하지 못했어요. {} 뒤에 다시 검색해 주세요.",
            wait_phrase(wait)
        ),
        SearchError::Busy(wait) => format!(
            "검색 서버가 {}초 뒤에 다시 요청해 달라고 해서 검색하지 못했어요.",
            wait.as_secs()
        ),
        SearchError::Read(why) => format!("검색 결과를 읽지 못했어요: {why}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fake::FakeNyaa, past_search::judge::State, test_world::World};
    use trss_core::Db;

    async fn service() -> (PastSearch, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        (PastSearch::new(SearchPace::new(db)), dir)
    }

    fn outcome() -> Outcome {
        Outcome {
            range: Range { from: 1, to: 1 },
            query: "Show".into(),
            preview: Preview::default(),
            notes: Vec::new(),
            first_full: false,
            extra_sent: 0,
            extra_needed: 0,
            storable: HashMap::from([(
                "k".to_owned(),
                Stored {
                    title: "Show - 01".into(),
                    link: "magnet:?xt=urn:btih:a".into(),
                    departed: false,
                },
            )]),
            rule_id: "r".into(),
        }
    }

    /// A task that never ends on its own, and an entry that holds it.
    fn entry(age: Duration, status: Status) -> (Entry, tokio::task::JoinHandle<()>) {
        let task = tokio::spawn(std::future::pending::<()>());
        let entry = Entry {
            rule_id: "r".into(),
            started: Instant::now().checked_sub(age).unwrap(),
            status,
            abort: Some(task.abort_handle()),
        };
        (entry, task)
    }

    async fn ended(task: tokio::task::JoinHandle<()>) -> bool {
        match tokio::time::timeout(Duration::from_secs(1), task).await {
            Ok(joined) => joined.unwrap_err().is_cancelled(),
            Err(_) => false,
        }
    }

    #[tokio::test]
    async fn a_search_dropped_for_its_age_is_aborted_as_by_every_other_removal() {
        let (service, _dir) = service().await;
        let (old, task) = entry(
            KEEP + Duration::from_secs(1),
            Status::Running { sent: 0, needed: 0 },
        );
        service.lock().searches.insert("old".into(), old);
        assert!(service.status("old").is_none());
        assert!(ended(task).await, "the dropped search's task still runs");
    }

    #[tokio::test]
    async fn a_search_past_its_keep_cannot_be_received_even_before_anything_sweeps() {
        let (service, _dir) = service().await;
        let (old, _task) = entry(
            KEEP + Duration::from_secs(1),
            Status::Done(Arc::new(outcome())),
        );
        service.lock().searches.insert("old".into(), old);
        assert_eq!(service.resolve("old", "r", "k"), Err(Resolve::Gone));

        let (fresh, _task) = entry(Duration::from_secs(1), Status::Done(Arc::new(outcome())));
        service.lock().searches.insert("fresh".into(), fresh);
        assert!(service.resolve("fresh", "r", "k").is_ok());
    }

    #[tokio::test]
    async fn a_result_is_resolved_only_from_a_finished_search_of_its_rule() {
        let (service, _dir) = service().await;
        let (running, _task1) = entry(Duration::ZERO, Status::Running { sent: 0, needed: 0 });
        let (failed, _task2) = entry(Duration::ZERO, Status::Failed("no".into()));
        let (done, _task3) = entry(Duration::ZERO, Status::Done(Arc::new(outcome())));
        {
            let mut registry = service.lock();
            registry.searches.insert("running".into(), running);
            registry.searches.insert("failed".into(), failed);
            registry.searches.insert("done".into(), done);
        }

        assert_eq!(service.resolve("none", "r", "k"), Err(Resolve::Gone));
        assert_eq!(service.resolve("running", "r", "k"), Err(Resolve::Running));
        assert_eq!(service.resolve("failed", "r", "k"), Err(Resolve::NoItem));
        assert_eq!(
            service.resolve("done", "other", "k"),
            Err(Resolve::OtherRule)
        );
        assert_eq!(service.resolve("done", "r", "nope"), Err(Resolve::NoItem));
        let stored = service.resolve("done", "r", "k").unwrap();
        assert_eq!(stored.title, "Show - 01");
        assert_eq!(stored.link, "magnet:?xt=urn:btih:a");
        assert!(!stored.departed);
    }

    /// The spec of a search of the first rule of `world`, on the tracker at
    /// `url`. The world holds the work folder, so it lives as long as the
    /// search.
    async fn spec_on(url: String, world: &World) -> Spec {
        let channel = world
            .ctx
            .channels
            .get_channel(&world.channel_id)
            .await
            .unwrap()
            .unwrap();
        Spec {
            channel: Channel { url, ..channel },
            rule: world.rule_of(0).await,
            query: "[SubsPlease] Show 1080p".into(),
            range: Range { from: 1, to: 2 },
            save_path: world.season.clone(),
            offset: 0,
            season: Some(1),
            settled: Vec::new(),
            listing: None,
            titles: Vec::new(),
            history_cut: false,
            redactor: Redactor::none(),
        }
    }

    /// The end of the search `id`: it is polled until it is not running.
    async fn finished(service: &PastSearch, id: &str) -> Status {
        for _ in 0..400 {
            match service.status(id) {
                Some((_, Status::Running { .. })) => {
                    tokio::time::sleep(Duration::from_millis(15)).await
                }
                Some((_, status)) => return status,
                None => panic!("the search is gone"),
            }
        }
        panic!("search hangs");
    }

    async fn serving() -> (PastSearch, tempfile::TempDir, FakeNyaa) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let service = PastSearch::new(SearchPace::new(db)).with_spacing(Duration::from_millis(5));
        (service, dir, FakeNyaa::start().await)
    }

    #[tokio::test]
    async fn a_new_search_of_a_rule_ends_the_one_before_it() {
        let (service, _dir, nyaa) = serving().await;
        nyaa.set_releases(&["[SubsPlease] Show - 01 (1080p) [AAAA0001].mkv".to_owned()]);
        let world = World::new().await;
        let rule_id = world.rule_of(0).await.id;

        let first = service.start(spec_on(nyaa.url("token"), &world).await);
        let second = service.start(spec_on(nyaa.url("token"), &world).await);

        assert_ne!(first, second);
        assert!(service.status(&first).is_none());
        assert_eq!(service.resolve(&first, &rule_id, "k"), Err(Resolve::Gone));
        let Status::Done(outcome) = finished(&service, &second).await else {
            panic!("the second search did not finish");
        };
        // The result of the search that ended is the later one's, of its rule.
        assert_eq!(outcome.rule_id, rule_id);
        assert_eq!(
            service.resolve(&second, "another rule", "k"),
            Err(Resolve::OtherRule)
        );
        assert_eq!(
            service.resolve(&second, &rule_id, "no such key"),
            Err(Resolve::NoItem)
        );
    }

    /// The sentence a tracker's `Retry-After` becomes ([`search_error`]), end
    /// to end from the answer: `client` tests the wait and the block.
    #[tokio::test]
    async fn a_tracker_that_asks_to_wait_fails_the_search_and_says_for_how_long() {
        let (service, _dir, nyaa) = serving().await;
        nyaa.refuse(Some((429, Some(30))));
        let world = World::new().await;

        let id = service.start(spec_on(nyaa.url("token"), &world).await);

        let Status::Failed(sentence) = finished(&service, &id).await else {
            panic!("the search did not fail");
        };
        assert!(sentence.contains("30초"), "{sentence}");
        assert_eq!(nyaa.queries().len(), 1);
    }

    /// The video of an episode the work folder holds is told by its CRC32
    /// from the file itself ([`file_crc32`]), whatever the revision's name
    /// says of its own.
    #[tokio::test]
    async fn a_video_of_unknown_version_is_told_by_the_crc_of_its_file_in_the_work_folder() {
        let (service, _dir, nyaa) = serving().await;
        let world = World::new().await;
        let old = b"episode 14, the video in the folder";
        std::fs::write(world.season.join("Show S01E14.mkv"), old).unwrap();
        let other = crate::test_world::crc(b"episode 14, a revision of another file");
        let v2 = |crc: &str| format!("[SubsPlease] Show - 14v2 (1080p) [{crc}].mkv");
        let v1 = format!(
            "[SubsPlease] Show - 14 (1080p) [{}].mkv",
            crate::test_world::crc(old)
        );
        let fifteen = "[SubsPlease] Show - 15 (1080p) [AAAA0015].mkv".to_owned();

        // (what the tracker lists, the state of the revision, whether the
        // search selects it)
        let searches = [
            (vec![v2(&other), fifteen], State::VersionUnknown, false),
            (
                vec![v2(&crate::test_world::crc(b"new")), v1],
                State::Replace,
                false,
            ),
            (vec![v2(&crate::test_world::crc(old))], State::Have, false),
        ];
        for (titles, state, selected) in searches {
            nyaa.set_releases(&titles);
            let mut spec = spec_on(nyaa.url("token"), &world).await;
            spec.range = Range { from: 14, to: 15 };
            let id = service.start(spec);
            let Status::Done(outcome) = finished(&service, &id).await else {
                panic!("the search did not finish");
            };
            let revision = outcome
                .preview
                .items
                .iter()
                .find(|item| item.title.contains("14v2"))
                .expect("the revision is listed");
            assert_eq!(
                (revision.state, revision.selected),
                (state, selected),
                "{titles:?}: {revision:?}"
            );
        }
    }
}
