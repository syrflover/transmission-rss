//! The world the collect-level tests of the receive line run in (ADR 0015:
//! a rule is tested in the crate that owns it). It holds what a worker
//! cycle would hold: the stores on one file-backed database (so SQL triggers
//! can fail a write on purpose), the fake Transmission acting on a temporary
//! media folder as Transmission does ([`FakeTransmission::on_disk`]), a fake
//! feed server, one channel with one rule saving to `Show/Season 01`, and a
//! manual clock.
//!
//! [`World::cycle`] runs the steps of the worker's `run_cycle` that decide
//! and act on items, in its order, against these fakes; the worker's own
//! tests keep what only a process does (locks, the order of cycles and
//! commands, concurrency, resume after an interruption). Release names,
//! hashes and contents are made up.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use trss_core::{
    commands::CommandStore, folder_locks::FolderLocks, settings::SettingsStore, Db, Millis,
};
use trss_library::{
    live::LiveWatch,
    store::{library::LibraryStore, seasons::SeasonStore},
};
use trss_transmission::{fake::FakeTransmission, Redactor, RenamePolicy};

use crate::{
    commands::rule_archive::work_folder::MovePolicy,
    context::{CollectContext, TransmissionLink},
    cycle::{self, JobOutcome},
    fake::FeedServer,
    feed,
    revisions::{self, Listing},
    season_link,
    store::{
        channels::{ChannelInput, ChannelStore, RuleInput},
        history::{HistoryItem, HistoryQuery, HistoryStore, MAX_PAGE_SIZE},
        revisions::{Revision, RevisionStore},
    },
};

/// A channel URL query value that must never show up in logs or history.
pub(crate) const SECRET: &str = "SECRETTOKEN0123456789";

/// The `dn` of a magnet link is the release name.
pub(crate) fn magnet(hash: &str, name: &str) -> String {
    let dn: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    format!("magnet:?xt=urn:btih:{hash}&dn={dn}")
}

/// A feed of `(hash, title)` items.
pub(crate) fn feed_xml(items: &[(&str, &str)]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>Show</title><link>https://feeds.example.test/show</link><description>made up</description>"#,
    );
    for (hash, title) in items {
        let link = magnet(hash, title).replace('&', "&amp;");
        xml.push_str(&format!(
            "<item><title>{title}</title><link>{link}</link><guid isPermaLink=\"false\">guid-{hash}</guid></item>"
        ));
    }
    xml.push_str("</channel></rss>");
    xml
}

/// The CRC32 of `bytes` as a release name writes it.
pub(crate) fn crc(bytes: &[u8]) -> String {
    format!("{:08X}", crc32fast::hash(bytes))
}

/// A fake Transmission acting on a temporary `media` folder (the collect
/// folder), a fake feed server, and one channel (`show` feed) whose rule
/// saves to `Show/Season 01`.
pub(crate) struct World {
    pub tr: FakeTransmission,
    pub feeds: FeedServer,
    pub ctx: CollectContext,
    _dir: TempDir,
    pub media: PathBuf,
    /// The rule's folder, `media/Show/Season 01`.
    pub season: PathBuf,
    pub channel_id: String,
    clock: Arc<AtomicI64>,
}

impl World {
    pub async fn new() -> World {
        World::with_match("[SubsPlease] Show - ").await
    }

    /// A world whose rule matches titles containing `text`.
    pub async fn with_match(text: &str) -> World {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let tr = FakeTransmission::start().await;
        let feeds = FeedServer::start().await;
        let media = dir.path().join("media");
        let season = media.join("Show").join("Season 01");
        std::fs::create_dir_all(&season).unwrap();
        let settings = SettingsStore::new(db.clone());
        settings
            .put_collection(0, media.to_str().unwrap().to_owned(), None)
            .await
            .unwrap();
        tr.on_disk(&media);
        let channels = ChannelStore::new(db.clone());
        let url = format!("{}?token={SECRET}", feeds.url("show"));
        let channel = channels
            .create_channel_with_rules(
                ChannelInput::new(url),
                vec![RuleInput {
                    r#match: Some(text.to_owned()),
                    directory: "Show/Season 01".to_owned(),
                    ..Default::default()
                }],
            )
            .await
            .unwrap();
        let library = LibraryStore::new(db.clone());
        let ctx = CollectContext {
            channels,
            settings,
            history: HistoryStore::new(db.clone()),
            revisions: RevisionStore::new(db.clone()),
            commands: CommandStore::new(db.clone()),
            seasons: SeasonStore::new(db),
            season_link: season_link::Memory::default(),
            library,
            live: LiveWatch::default(),
            transmission: TransmissionLink {
                url: tr.url().parse().unwrap(),
                http: trss_transmission::http_client(Duration::from_secs(5)).unwrap(),
            },
            http: feed::client().unwrap(),
            rename: RenamePolicy {
                delay: Duration::from_millis(5),
                attempts: 3,
            },
            moves: MovePolicy::default(),
            redactor: Redactor::none(),
            folders: FolderLocks::new(),
        };
        World {
            tr,
            feeds,
            ctx,
            media,
            season,
            channel_id: channel.channel.id,
            _dir: dir,
            clock: Arc::new(AtomicI64::new(1_000_000)),
        }
    }

    /// The manual clock.
    pub fn now(&self) -> Millis {
        self.clock.load(Ordering::SeqCst)
    }

    // --- the feeds and the folder ------------------------------------------------------

    /// The `show` feed holds `items`.
    pub fn feed(&self, items: &[(&str, &str)]) {
        self.feeds.set_xml("show", &feed_xml(items));
    }

    /// A second channel whose rule saves the same release to the same folder,
    /// reading the feed `path` (empty until set).
    pub async fn second_channel(&self, path: &str) -> String {
        self.feeds.set_xml(path, &feed_xml(&[]));
        let url = format!("{}?token={SECRET}", self.feeds.url(path));
        self.ctx
            .channels
            .create_channel_with_rules(
                ChannelInput::new(url),
                vec![RuleInput {
                    r#match: Some("[SubsPlease] Show - ".to_owned()),
                    directory: "Show/Season 01".to_owned(),
                    ..Default::default()
                }],
            )
            .await
            .unwrap()
            .channel
            .id
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.season.join(name)
    }

    /// The names in the season folder, sorted.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.season)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    // --- a cycle -----------------------------------------------------------------------

    /// One collection cycle at the manual clock's time: the channels are read,
    /// the items judged, the revisions' items left to their replacements, the
    /// selected items added and named, the replacements carried on, and the
    /// torrents of items that left the feeds removed, in the order of the
    /// worker's `run_cycle`. Its session settings, status board records,
    /// episode offsets and the gate it shares with the commands are not part
    /// of it.
    pub async fn cycle(&self) {
        let at = self.now();
        let ctx = &self.ctx;
        let cancel = CancellationToken::new();
        let snapshot = ctx.channels.list_channels_with_rules().await.unwrap();
        let plans = cycle::make_plans(ctx, snapshot, &self.media).await;
        let mut redactor = ctx.redactor.clone();
        for plan in &plans {
            redactor.extend(&plan.redactor());
        }

        let mut jobs = Vec::new();
        let mut present = Vec::new();
        let mut unread = Vec::new();
        let mut read = 0;
        for plan in &plans {
            match feed::fetch(&ctx.http, &plan.channel.url).await {
                Ok(feed) => {
                    read += 1;
                    let judged = cycle::judge_feed(ctx, plan, &feed, Some(&self.media), at).await;
                    jobs.extend(judged.jobs);
                    present.extend(judged.present);
                }
                Err(_) => unread.push(plan.channel.id.clone()),
            }
        }
        let (jobs, _) = cycle::leave_revisions(ctx, jobs, at).await;

        let listing = Arc::new(Listing::new());
        let mut kept = HashSet::new();
        let mut removable = read > 0;
        for job in jobs {
            let (outcome, _) = cycle::process_job(
                ctx.clone(),
                job,
                at,
                redactor.clone(),
                cancel.clone(),
                listing.clone(),
            )
            .await;
            match outcome {
                JobOutcome::Held { hash, .. } => {
                    kept.insert(hash);
                }
                JobOutcome::Failed { unconfirmed: true } => removable = false,
                _ => {}
            }
        }
        revisions::advance(&ctx.revision_work(), &ctx.folders, at, &redactor, &cancel).await;
        if removable {
            cycle::remove_departed(ctx, kept, present, unread, &redactor).await;
        }
    }

    // --- history and the replacements --------------------------------------------------

    pub async fn history_items(&self) -> Vec<HistoryItem> {
        self.ctx
            .history
            .list(HistoryQuery {
                limit: MAX_PAGE_SIZE,
                ..Default::default()
            })
            .await
            .unwrap()
            .items
    }

    /// The history item titled `title`.
    pub async fn item(&self, title: &str) -> HistoryItem {
        self.history_items()
            .await
            .into_iter()
            .find(|i| i.title == title)
            .unwrap_or_else(|| panic!("no history item {title}"))
    }

    /// The replacement row of the item titled `title`, if it has one.
    pub async fn row_if(&self, title: &str) -> Option<Revision> {
        let item = self.item(title).await;
        self.ctx.revisions.by_item(item.id).await.unwrap()
    }

    /// The replacement row of the item titled `title`.
    pub async fn row_of(&self, title: &str) -> Revision {
        self.row_if(title).await.expect("a revision row")
    }

    /// The replacements the to-do source lists as `받기 실패`.
    pub async fn failures(&self) -> Vec<Revision> {
        self.ctx.revisions.failures().await.unwrap()
    }

    // --- Transmission ------------------------------------------------------------------

    /// Transmission has all of the torrent `hash`.
    pub fn complete(&self, hash: &str) {
        self.tr.finish(hash);
        self.tr.set_status(hash, 6);
    }

    /// How many times the torrent `hash` was added.
    pub fn added(&self, hash: &str) -> usize {
        self.tr
            .calls_of("torrent-add")
            .iter()
            .filter(|c| c.args["filename"].as_str().unwrap().contains(hash))
            .count()
    }

    /// Whether the torrent `hash` was removed.
    pub fn removed(&self, hash: &str) -> bool {
        self.tr
            .calls_of("torrent-remove")
            .iter()
            .any(|c| c.args["ids"] == serde_json::json!([hash]))
    }
}

/// The text of `path`'s bytes.
pub(crate) fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}
