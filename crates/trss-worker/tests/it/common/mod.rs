//! Test support shared by the worker tests: a harness that wires a worker to
//! the fake Transmission RPC server (`trss_transmission::fake`), the fake feed
//! and nyaa servers (`trss_collect::fake`) and a temporary app database.
//! Their names are re-exported here so the test files import them from one
//! place.

#![allow(dead_code)]

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{http::StatusCode, Router};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;
use trss_collect::store::{
    channels::{ChannelInput, ChannelStore, ChannelWithRules, RuleInput},
    history::{HistoryItem, HistoryQuery, HistoryStore, MAX_PAGE_SIZE},
};
use trss_core::{lock_path_for, settings::SettingsStore, CycleLock, Db};
use trss_transmission::RenamePolicy;
use trss_web::AppState;
use trss_worker::{Worker, WorkerEnv};

/// The collect folder every harness starts with. Test channels are given by
/// the folder they used to have as a base folder (`/media/anime`), which
/// [`Harness::add_channel`] turns into a rule-directory prefix under it, so the
/// save paths the tests expect stay the same text.
pub const COLLECT_FOLDER: &str = "/media";

pub use trss_collect::fake::{FakeNyaa, FeedServer};
pub use trss_transmission::{
    fake::{FakeTorrent, FakeTransmission},
    BOT_LABEL,
};

pub const FEED_A: &str = include_str!("../../fixtures/worker_feed_a.xml");
pub const FEED_B: &str = include_str!("../../fixtures/worker_feed_b.xml");
pub const CHANNELS_YAML: &str = include_str!("../../fixtures/worker_channels.yml");

// --- harness -----------------------------------------------------------------------

/// A channel URL query value that must never show up in logs or history.
pub const SECRET: &str = "SECRETTOKEN0123456789";

pub struct Harness {
    pub dir: tempfile::TempDir,
    pub db: Db,
    pub channels: ChannelStore,
    pub history: HistoryStore,
    pub tr: FakeTransmission,
    pub feeds: FeedServer,
    pub clock: Arc<AtomicI64>,
}

impl Harness {
    /// A harness whose collect folder is [`COLLECT_FOLDER`].
    pub async fn new() -> Harness {
        let harness = Harness::without_collect_folder().await;
        SettingsStore::new(harness.db.clone())
            .put_collection(0, COLLECT_FOLDER.to_owned(), None)
            .await
            .unwrap();
        harness
    }

    /// A harness on a database with no collect folder set, as a fresh one is.
    pub async fn without_collect_folder() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let feeds = FeedServer::start().await;
        feeds.set_xml("feed-a", FEED_A);
        feeds.set_xml("feed-b", FEED_B);
        Harness {
            channels: ChannelStore::new(db.clone()),
            history: HistoryStore::new(db.clone()),
            tr: FakeTransmission::start().await,
            feeds,
            db,
            dir,
            clock: Arc::new(AtomicI64::new(1_000_000)),
        }
    }

    pub fn db_path(&self) -> std::path::PathBuf {
        self.dir.path().join("app.db")
    }

    /// Waits until a worker stopped midway (its task aborted) has let go of
    /// the lock between processes. It lets go only after its last heartbeat,
    /// in a task of its own, and every worker [`Harness::worker`] builds is
    /// another process to it, which finds the lock taken until then and
    /// answers `Busy`. A restarted process finds it free at once: the
    /// operating system let go of the stopped one's.
    pub async fn wait_lock_free(&self) {
        let lock = lock_path_for(&self.db_path());
        for _ in 0..1000 {
            if CycleLock::try_acquire(&lock).unwrap().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the stopped worker never let go of its lock");
    }

    pub fn worker_env(&self) -> WorkerEnv {
        self.worker_env_with(&[])
    }

    /// The worker settings as environment variables would give them, plus `extra`.
    pub fn worker_env_with(&self, extra: &[(&str, &str)]) -> WorkerEnv {
        let tr_url = self.tr.url();
        let extra: HashMap<String, String> = extra
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        WorkerEnv::from_lookup(move |key| match key {
            "TRANSMISSION_URL" => Some(tr_url.clone()),
            other => extra.get(other).cloned(),
        })
        .unwrap()
    }

    /// A worker on this harness's database and fakes, with a fast rename
    /// policy, a manual clock and no minimum gap between cycles.
    pub fn worker(&self) -> Worker {
        self.worker_with_db(self.db.clone())
    }

    pub fn worker_with_db(&self, db: Db) -> Worker {
        self.worker_with(db, &self.worker_env())
    }

    pub fn worker_with(&self, db: Db, env: &WorkerEnv) -> Worker {
        let clock = self.clock.clone();
        Worker::new(db, env, lock_path_for(&self.db_path()))
            .unwrap()
            .with_clock(Arc::new(move || clock.load(Ordering::SeqCst)))
            .with_rename_policy(RenamePolicy {
                delay: Duration::from_millis(5),
                attempts: 3,
            })
            .with_min_gap(Duration::ZERO)
    }

    /// Moves the manual clock forward.
    pub fn advance(&self, millis: i64) {
        self.clock.fetch_add(millis, Ordering::SeqCst);
    }

    pub fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    /// Adds a channel for one of the fake feeds. Every query value of the URL
    /// is secret, as for a channel added in the app.
    ///
    /// `base_dir` is the folder the channel's rules' directories are under, as
    /// channels used to have: it must be [`COLLECT_FOLDER`] or inside it, and
    /// what lies below the collect folder is put in front of each rule's
    /// directory, so the rule saves to `base_dir` + directory as before.
    pub async fn add_channel(
        &self,
        feed_path: &str,
        base_dir: &str,
        excludes: &[&str],
        mut rules: Vec<RuleInput>,
    ) -> ChannelWithRules {
        let url = format!("{}?filter=1080p&token={SECRET}", self.feeds.url(feed_path));
        let mut input = ChannelInput::new(url);
        input.excludes = excludes.iter().map(|s| s.to_string()).collect();
        let below = std::path::Path::new(base_dir)
            .strip_prefix(COLLECT_FOLDER)
            .unwrap_or_else(|_| panic!("{base_dir} is not inside {COLLECT_FOLDER}"))
            .to_string_lossy()
            .into_owned();
        for rule in &mut rules {
            rule.directory = trss_core::folders::prefixed(&below, &rule.directory);
        }
        self.channels
            .create_channel_with_rules(input, rules)
            .await
            .unwrap()
    }

    pub async fn history_items(&self) -> Vec<HistoryItem> {
        self.history
            .list(HistoryQuery {
                limit: MAX_PAGE_SIZE,
                ..Default::default()
            })
            .await
            .unwrap()
            .items
    }

    pub async fn item(&self, title_part: &str) -> HistoryItem {
        let items = self.history_items().await;
        let mut found = items.into_iter().filter(|i| i.title.contains(title_part));
        let item = found
            .next()
            .unwrap_or_else(|| panic!("no history item with {title_part:?}"));
        assert!(
            found.next().is_none(),
            "several history items with {title_part:?}"
        );
        item
    }
}

// --- the real web API, in process ---------------------------------------------------

/// The `/api` router on the harness's own database handle, called without a
/// socket. Requests go through the same handlers and store as `trss-web`.
pub struct WebApi {
    router: Router,
}

impl WebApi {
    pub fn new(db: Db) -> WebApi {
        WebApi {
            router: Router::new().nest(
                "/api",
                trss_web::api::router().with_state(AppState::new(db)),
            ),
        }
    }

    /// Sends one request; returns the status, the raw body text and its JSON
    /// (`null` when the body is not JSON).
    pub async fn call(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, String, Value) {
        let mut request = axum::http::Request::builder().method(method).uri(uri);
        let body = match body {
            Some(json) => {
                request = request.header("content-type", "application/json");
                axum::body::Body::from(json.to_string())
            }
            None => axum::body::Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let json = serde_json::from_str(&text).unwrap_or(Value::Null);
        (status, text, json)
    }
}

impl Harness {
    /// The channels API on this harness's database.
    pub fn web_api(&self) -> WebApi {
        WebApi::new(self.db.clone())
    }
}

pub fn rule(match_text: &str, directory: &str) -> RuleInput {
    RuleInput {
        r#match: Some(match_text.to_owned()),
        directory: directory.to_owned(),
        ..Default::default()
    }
}

/// The rules of the first channel of `worker_channels.yml`, as stored rules.
pub fn feed_a_rules() -> Vec<RuleInput> {
    vec![
        rule("[SubsPlease] Sayonara Lara - ", "Sayonara Lara/Season 01"),
        RuleInput {
            case_insensitive: true,
            episode: -12,
            ..rule("sono bisque doll", "Sono Bisque Doll/Season 02")
        },
        rule("Sono Bisque Doll", "Sono Bisque Doll (overlap)"),
        RuleInput {
            episode: -24,
            ..rule(
                "[SubsPlease] Tensei Shitara Slime Datta Ken",
                "Slime/Season 04",
            )
        },
    ]
}

impl WebApi {
    /// The `/api` router over `state`.
    pub fn with_state(state: AppState) -> WebApi {
        WebApi {
            router: Router::new().nest("/api", trss_web::api::router().with_state(state)),
        }
    }
}
