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
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use trss_anilist::{Entry, FuzzyDate};
use trss_anissia::Anime;
use trss_core::{
    commands::{Accepted, Command, CommandStore, NewCommand},
    folder_locks::FolderLocks,
    settings::SettingsStore,
    Clock, Db, Millis,
};
use trss_library::{
    discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead},
    live::LiveWatch,
    store::{library::LibraryStore, seasons::SeasonStore},
};
use trss_transmission::{fake::FakeTransmission, Redactor, RenamePolicy};

use crate::{
    commands::{
        episode_undo::{self, EpisodeUndo, Finished as UndoFinished, Retry as UndoRetry},
        receive_once::{self, Finished, ReceiveOnce, Retry},
        rule_archive::work_folder::MovePolicy,
    },
    context::{CollectContext, TransmissionLink},
    cycle::{self, JobOutcome},
    episode_offset::{gather, Basis},
    fake::FeedServer,
    feed,
    revisions::{self, Listing},
    season_link,
    store::{
        channels::{
            ChannelInput, ChannelStore, EpisodeMark, NewSubscription, Rule, RuleInput, SubtitleMode,
        },
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
    pub dir: TempDir,
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
        World::with_rules(vec![RuleInput {
            r#match: Some(text.to_owned()),
            directory: "Show/Season 01".to_owned(),
            ..Default::default()
        }])
        .await
    }

    /// A world whose channel has no rule: a test adds the rules it needs,
    /// such as [`World::subscribe`].
    pub async fn bare() -> World {
        World::with_rules(Vec::new()).await
    }

    async fn with_rules(rules: Vec<RuleInput>) -> World {
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
            .create_channel_with_rules(ChannelInput::new(url), rules)
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
            dir,
            clock: Arc::new(AtomicI64::new(1_000_000)),
        }
    }

    /// The manual clock.
    pub fn now(&self) -> Millis {
        self.clock.load(Ordering::SeqCst)
    }

    /// Moves the manual clock forward.
    pub fn advance(&self, millis: i64) {
        self.clock.fetch_add(millis, Ordering::SeqCst);
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

    /// The database file the stores share.
    pub fn db_path(&self) -> PathBuf {
        self.dir.path().join("app.db")
    }

    /// Runs `sql` on the database from another connection.
    pub fn sql(&self, sql: &str) {
        rusqlite::Connection::open(self.db_path())
            .unwrap()
            .execute_batch(sql)
            .unwrap();
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
    /// worker's `run_cycle`; a new season's rule gets its episode offset
    /// ([`cycle::settle_offsets`]) before its first item is added. Its session
    /// settings, status board records and the gate it shares with the
    /// commands are not part of it.
    pub async fn cycle(&self) {
        let at = self.now();
        let ctx = &self.ctx;
        let cancel = CancellationToken::new();
        let snapshot = ctx.channels.list_channels_with_rules().await.unwrap();
        let open_rules = cycle::open_rules(&snapshot);
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
        let (mut jobs, _) = cycle::leave_revisions(ctx, jobs, at).await;
        cycle::settle_offsets(ctx, self.media.to_str().unwrap(), &open_rules, &mut jobs).await;

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

    // --- a subscription to a new season ------------------------------------------------

    /// A subscription of the channel to the anime `anime_no`, matching titles
    /// that contain `phrase`, saving into `directory` with the offset
    /// `episode` in its field. The channel's feed is read by a cycle after
    /// [`World::feed_after_other`] has given it something else to hold.
    pub async fn subscribe(
        &self,
        phrase: &str,
        directory: &str,
        anime_no: i64,
        episode: i64,
    ) -> Rule {
        self.ctx
            .channels
            .create_subscription_rule(
                &self.channel_id,
                RuleInput {
                    r#match: Some(phrase.to_owned()),
                    directory: directory.to_owned(),
                    episode,
                    ..Default::default()
                },
                NewSubscription {
                    anime: Anime {
                        anime_no,
                        subject: phrase.to_owned(),
                        original_subject: None,
                        week: 4,
                        air_time: Some("23:00".to_owned()),
                        start_date: Some("2026-10-08".to_owned()),
                        end_date: None,
                        status: "ON".to_owned(),
                        fetched_at: self.now(),
                    },
                    subtitles: SubtitleMode::Undecided,
                    creator: None,
                    subscribed_at: self.now(),
                },
            )
            .await
            .unwrap()
    }

    /// The `show` feed holds `items` after a release of another work: a
    /// channel read once holds something already, so that what comes later is
    /// not what the feed held at the first read.
    pub fn feed_after_other(&self, items: &[(&str, &str)]) {
        let other = (OTHER_HASH, OTHER_TITLE);
        let all: Vec<(&str, &str)> = std::iter::once(other)
            .chain(items.iter().copied())
            .collect();
        self.feed(&all);
    }

    /// The library of the collect folder, holding `Show` with these seasons
    /// and the videos named in each.
    pub async fn library_of(&self, seasons: &[(u32, &[&str])]) -> Place {
        Place::on(
            self.ctx.library.clone(),
            self.ctx.seasons.clone(),
            self.media.to_str().unwrap(),
            seasons,
        )
        .await
    }

    /// The `show` feed holds these episodes of `Show` after a release of
    /// another work ([`World::feed_after_other`]).
    pub fn feed_shows(&self, numbers: &[u32]) {
        let releases: Vec<(String, String)> = numbers
            .iter()
            .map(|n| (show_hash(*n), show_title(*n)))
            .collect();
        let items: Vec<(&str, &str)> = releases
            .iter()
            .map(|(hash, title)| (hash.as_str(), title.as_str()))
            .collect();
        self.feed_after_other(&items);
    }

    /// A cycle five minutes after the last one.
    pub async fn cycle_later(&self) {
        self.advance(300_000);
        self.cycle().await;
    }

    /// The names Transmission's torrents have now, sorted.
    pub fn torrent_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tr.torrents().into_iter().map(|t| t.name).collect();
        names.sort();
        names
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

    /// The one history item whose title contains `part`.
    pub async fn item_containing(&self, part: &str) -> HistoryItem {
        let mut found = self
            .history_items()
            .await
            .into_iter()
            .filter(|i| i.title.contains(part));
        let item = found
            .next()
            .unwrap_or_else(|| panic!("no history item with {part:?}"));
        assert!(
            found.next().is_none(),
            "several history items with {part:?}"
        );
        item
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

    // --- the rule's episode offset -------------------------------------------------------

    /// The rule as stored now.
    pub async fn stored_rule(&self, rule: &Rule) -> Rule {
        self.ctx.channels.get_rule(&rule.id).await.unwrap().unwrap()
    }

    /// What is kept of the app's decision about the rule's offset.
    pub async fn mark_of(&self, rule: &Rule) -> EpisodeMark {
        self.ctx
            .channels
            .episode_marks(vec![rule.id.clone()])
            .await
            .unwrap()
            .remove(&rule.id)
            .unwrap()
    }

    // --- 되돌리기 -----------------------------------------------------------------------

    /// Accepts `되돌리기` of the automatic offset `episode` of `rule` as the
    /// command `id`, and claims it as the worker does.
    pub async fn start_undo(&self, rule: &Rule, id: &str, episode: i64) -> Command {
        let payload = EpisodeUndo {
            rule_id: rule.id.clone(),
            episode,
        };
        let new = NewCommand {
            id: id.to_owned(),
            kind: episode_undo::KIND.to_owned(),
            payload: payload.canonical(),
            subject: Some(payload.subject()),
        };
        match self.ctx.commands.accept(new, self.now()).await.unwrap() {
            Accepted::Created(_) => {}
            other => panic!("the command was not stored: {other:?}"),
        }
        self.ctx
            .commands
            .claim_next(self.now())
            .await
            .unwrap()
            .expect("the command waits")
    }

    /// Runs `command` as the worker does and writes its end. A [`Retry`]
    /// leaves the command running, to be run again.
    pub async fn run_undo(&self, command: &Command) -> Result<UndoFinished, UndoRetry> {
        let clock = self.clock.clone();
        let clock: Clock = Arc::new(move || clock.load(Ordering::SeqCst));
        let finished = episode_undo::run(&self.ctx.undo(), command, &clock).await?;
        self.ctx
            .commands
            .finish(
                &command.id,
                finished.state,
                finished.outcome.clone(),
                self.now(),
            )
            .await
            .unwrap();
        Ok(finished)
    }

    /// `되돌리기` of `episode` as the command `id`, run once: how it ended.
    pub async fn undo(&self, rule: &Rule, id: &str, episode: i64) -> UndoFinished {
        let command = self.start_undo(rule, id, episode).await;
        self.run_undo(&command).await.expect("the undo ran")
    }

    // --- 다시 받기 ---------------------------------------------------------------------

    /// Accepts `다시 받기` of `item_id` as the command `id`, as the web does,
    /// and claims it as the worker does.
    pub async fn start_retry(&self, item_id: i64, id: &str) -> Command {
        self.start_receive(ReceiveOnce::new(item_id), id).await
    }

    /// `이 규칙으로 받기` of `item_id` for the rule `rule_id` as the command
    /// `id`, run once.
    pub async fn receive_for(
        &self,
        item_id: i64,
        rule_id: &str,
        id: &str,
    ) -> Result<Finished, Retry> {
        let command = self
            .start_receive(ReceiveOnce::by_rule(item_id, rule_id), id)
            .await;
        self.run_command(&command).await
    }

    async fn start_receive(&self, payload: ReceiveOnce, id: &str) -> Command {
        let new = NewCommand {
            id: id.to_owned(),
            kind: receive_once::KIND.to_owned(),
            payload: payload.canonical(),
            subject: Some(payload.subject()),
        };
        match self.ctx.commands.accept(new, self.now()).await.unwrap() {
            Accepted::Created(_) => {}
            other => panic!("the command was not stored: {other:?}"),
        }
        self.ctx
            .commands
            .claim_next(self.now())
            .await
            .unwrap()
            .expect("the command waits")
    }

    /// Runs `command` as the worker does and, when it ends, writes the end:
    /// the add, the rename and the labels, then the command's state. A
    /// [`Retry`] leaves the command running, to be run again.
    pub async fn run_command(&self, command: &Command) -> Result<Finished, Retry> {
        let clock = self.clock.clone();
        let finished = receive_once::run(
            &self.ctx.receive(),
            command,
            move || clock.load(Ordering::SeqCst),
            &CancellationToken::new(),
        )
        .await?;
        let (state, outcome) = (finished.state, finished.outcome.clone());
        let ended = if finished.add_unconfirmed {
            self.ctx
                .commands
                .finish_with_unconfirmed_add(&command.id, state, outcome, self.now())
                .await
        } else {
            self.ctx
                .commands
                .finish(&command.id, state, outcome, self.now())
                .await
        };
        ended.unwrap();
        Ok(finished)
    }

    /// `다시 받기` of `item_id` as the command `id`, run once.
    pub async fn retry(&self, item_id: i64, id: &str) -> Result<Finished, Retry> {
        let command = self.start_retry(item_id, id).await;
        self.run_command(&command).await
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

/// Episode `n` of `Show`, released by SubsPlease.
pub(crate) fn show_title(n: u32) -> String {
    format!("[SubsPlease] Show - {n:02} (1080p) [ABCD{n:04}].mkv")
}

/// The hash of episode `n` of `Show`.
pub(crate) fn show_hash(n: u32) -> String {
    format!("dddd{n:036}")
}

/// The hash and title of a release of another work, which a channel read once
/// holds already.
pub(crate) const OTHER_HASH: &str = "9999000000000000000000000000000000000001";
pub(crate) const OTHER_TITLE: &str = "[SubsPlease] Other - 01 (1080p) [ZZZZ0001].mkv";

/// An AniList entry with this episode count.
pub(crate) fn entry(id: i64, episodes: Option<u32>) -> Entry {
    Entry {
        id,
        romaji: None,
        english: None,
        native: None,
        format: None,
        status: None,
        episodes,
        start: FuzzyDate::default(),
        end: FuzzyDate::default(),
        studios: Vec::new(),
        genres: Vec::new(),
        description: None,
        airing: Vec::new(),
        korean_titles: Vec::new(),
        sequels: Vec::new(),
        fetched_at: 1,
    }
}

/// The library holding `Show` under a collect folder, with AniList entries
/// linked to its seasons: what the episode offset reads of the earlier
/// seasons ([`gather`]).
pub(crate) struct Place {
    pub library: LibraryStore,
    pub seasons: SeasonStore,
    pub work: String,
    folder: String,
}

impl Place {
    /// `Show` under `/shows`, in a database of its own, with the given
    /// seasons, each with the videos named.
    pub async fn new(seasons: &[(u32, &[&str])]) -> Place {
        let db = Db::open_blocking(":memory:").unwrap();
        Place::on(
            LibraryStore::new(db.clone()),
            SeasonStore::new(db),
            "/shows",
            seasons,
        )
        .await
    }

    /// `Show` under the collect folder `folder`, in these stores.
    pub async fn on(
        library: LibraryStore,
        seasons: SeasonStore,
        folder: &str,
        held: &[(u32, &[&str])],
    ) -> Place {
        let files = held
            .iter()
            .flat_map(|(season, episodes)| {
                episodes.iter().map(move |episode| EpisodeFile {
                    path: format!("Season {season:02}/Show S{season:02}E{episode}.mkv"),
                    kind: FileKind::Video,
                    season: *season,
                    episode: (*episode).to_owned(),
                })
            })
            .collect();
        let scan = Scan {
            works: vec![WorkRead::Read(ScannedWork {
                dir_name: "Show".into(),
                seasons: held.iter().map(|(s, _)| *s).collect::<BTreeSet<_>>(),
                files,
                unrecognized: Vec::new(),
            })],
        };
        let (added, _) = library
            .add_folder(folder.to_owned(), scan, 100, &[])
            .await
            .unwrap();
        let work = library.works(&added.id).await.unwrap().remove(0).id;
        Place {
            library,
            seasons,
            work,
            folder: folder.to_owned(),
        }
    }

    /// Links entries with these counts to the season.
    pub async fn link(&self, season: u32, counts: &[Option<u32>]) {
        let mut ids = Vec::new();
        for (index, count) in counts.iter().enumerate() {
            let id = i64::from(season) * 10 + index as i64;
            self.seasons.put_entry(entry(id, *count)).await.unwrap();
            ids.push(id);
        }
        let link = self.seasons.link(&self.work, season).await.unwrap();
        self.seasons
            .set_links(&self.work, season, link.version, ids)
            .await
            .unwrap();
    }

    pub async fn basis(&self, rule: &Rule) -> Option<Basis> {
        gather(
            &self.library,
            &self.seasons,
            &format!("{}/", self.folder),
            None,
            rule,
        )
        .await
        .unwrap()
    }
}

/// The text of `path`'s bytes.
pub(crate) fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}
