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

use super::{
    client::{wait_phrase, SearchClient, SearchError, REQUEST_SPACING},
    judge::{judge, Preview, Range, Result as Judged},
    run::{run, Limits},
    world,
};
use crate::{
    revision::file_crc32,
    store::{
        channels::{Channel, Rule},
        history::HistoryItem,
        search_pace::SearchPace,
    },
    transmission::Redactor,
    worker::{feed::FeedItem, plan::picks},
};

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
    /// The channel's items history says Transmission holds.
    pub settled: Vec<HistoryItem>,
    /// Every title history holds for the channel.
    pub titles: Vec<String>,
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
        let registry = self.lock();
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
            titles,
            redactor,
        } = spec;

        let files = tokio::task::spawn_blocking(move || world::read_folder(&save_path))
            .await
            .map_err(|_| "작품 폴더를 읽는 중 오류가 났어요.".to_owned())?
            .map_err(|err| format!("작품 폴더를 읽지 못했어요: {}", err.kind()))?;
        let world = world::build(offset, season, files, &rule.id, &settled, &titles);

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
                        },
                    )
                })
            })
            .collect();
        Ok(Outcome {
            range,
            query,
            preview,
            notes: found.notes,
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
    use crate::store::Db;

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
}
