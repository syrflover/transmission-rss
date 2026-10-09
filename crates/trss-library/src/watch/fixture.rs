//! What the tests of the reading of watch folders share (`watch`,
//! `watch_rescan`, `live`): a [`WatchContext`] on a database file, temporary
//! folders for watch folders, and a manual clock, so that the time a file gets
//! is exact. File system times are set to values that must never be read.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use trss_core::{
    folder_locks::FolderLocks, heartbeat::HeartbeatStore, lock_path_for, settings::SettingsStore,
    Clock, Db, Millis, WorkerLock,
};

use crate::{
    discovery,
    live::{LiveConfig, LiveWatch},
    store::library::{LibraryStore, WatchFolder, WorkRecord},
    watch::{self, ScanCaches, WatchContext},
};

pub(crate) struct Fixture {
    pub ctx: WatchContext,
    pub db: Db,
    pub dir: TempDir,
    /// A folder under `dir` that watch folders go in.
    pub media: PathBuf,
    clock: Arc<AtomicI64>,
}

impl Fixture {
    /// A worker that is not watching: every cycle reads every folder.
    pub async fn new() -> Fixture {
        Fixture::with_live(LiveConfig::default()).await
    }

    /// A context whose [`LiveWatch`] has `config`, to be started with
    /// [`Fixture::start_watching`].
    pub async fn with_live(config: LiveConfig) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let media = dir.path().join("media");
        fs::create_dir_all(&media).unwrap();
        Fixture {
            ctx: WatchContext {
                library: LibraryStore::new(db.clone()),
                settings: SettingsStore::new(db.clone()),
                folders: FolderLocks::new(),
                scan_cache: ScanCaches::default(),
                live: LiveWatch::new(config),
            },
            db,
            dir,
            media,
            clock: Arc::new(AtomicI64::new(1_000_000)),
        }
    }

    // --- the manual clock -----------------------------------------------------------------

    pub fn clock(&self) -> Clock {
        let clock = self.clock.clone();
        Arc::new(move || clock.load(Ordering::SeqCst))
    }

    pub fn now(&self) -> Millis {
        self.clock.load(Ordering::SeqCst)
    }

    pub fn advance(&self, millis: Millis) {
        self.clock.fetch_add(millis, Ordering::SeqCst);
    }

    // --- folders ----------------------------------------------------------------------------

    /// A folder called `name` under the media folder.
    pub fn folder(&self, name: &str) -> PathBuf {
        let path = self.media.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// Registers the folder at `path` as the web does: one reading, recorded
    /// with the folder, at the clock's time.
    pub async fn register(&self, path: &Path) -> WatchFolder {
        let scan = discovery::scan(path).unwrap();
        let registered = self.ctx.library.folders().await.unwrap();
        let (folder, _) = self
            .ctx
            .library
            .add_folder(text(path), scan, self.now(), &registered)
            .await
            .unwrap();
        folder
    }

    pub async fn folder_at(&self, path: &Path) -> Option<WatchFolder> {
        self.ctx
            .library
            .folders()
            .await
            .unwrap()
            .into_iter()
            .find(|f| f.path == text(path))
    }

    pub async fn stored(&self, folder: &WatchFolder) -> WatchFolder {
        self.ctx.library.folder(&folder.id).await.unwrap().unwrap()
    }

    pub async fn works(&self, folder: &WatchFolder) -> Vec<WorkRecord> {
        self.ctx.library.works(&folder.id).await.unwrap()
    }

    pub async fn work(&self, folder: &WatchFolder, name: &str) -> WorkRecord {
        self.find(folder, name)
            .await
            .unwrap_or_else(|| panic!("no work {name}"))
    }

    pub async fn find(&self, folder: &WatchFolder, name: &str) -> Option<WorkRecord> {
        self.works(folder)
            .await
            .into_iter()
            .find(|w| w.dir_name == name)
    }

    /// The `added_at` of a file of a work, once the library has the file.
    pub async fn added_at(
        &self,
        folder: &WatchFolder,
        work: &str,
        file: &str,
    ) -> Option<Option<Millis>> {
        let work = self.find(folder, work).await?;
        let added = work.files().get(file).map(|f| f.added_at);
        added
    }

    // --- readings -----------------------------------------------------------------------------

    /// What a worker cycle does with the watch folders.
    pub async fn scan_all(&self) {
        watch::scan_all(&self.ctx, &self.clock(), &CancellationToken::new()).await;
    }

    /// Starts watching the registered folders, as a running worker does.
    pub async fn start_watching(&self) {
        let lock = WorkerLock::new(
            lock_path_for(&self.dir.path().join("app.db")),
            HeartbeatStore::new(self.db.clone()),
            self.clock(),
            Duration::from_secs(15),
        );
        self.ctx.live.start(self.ctx.clone(), lock, self.clock());
        self.ctx.live.sync_folders().await;
    }
}

pub(crate) fn text(path: impl AsRef<Path>) -> String {
    path.as_ref().to_str().unwrap().to_owned()
}

pub(crate) fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "x").unwrap();
}

/// Gives every directory under `root` (and `root`) a modification time in 2001,
/// as a folder nobody has touched for a long time has. The periodic scan does
/// not trust a directory changed moments before it listed it, so the trees
/// these tests build must look settled before it can skip them.
pub(crate) fn age_dirs(root: &Path) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() && !path.is_symlink() {
            age_dirs(&path);
        }
    }
    fs::File::open(root)
        .unwrap()
        .set_modified(long_ago())
        .unwrap();
}

/// Sets a file's modification time to the year 2001, which no record may show.
pub(crate) fn age(path: &Path) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(long_ago())
        .unwrap();
}

fn long_ago() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000)
}

/// The Lycoris Recoil tree of the worker's first tests: two seasons, three
/// videos and a subtitle.
pub(crate) fn lycoris(root: &Path) {
    for file in [
        "Lycoris Recoil/Season 01/Lycoris Recoil S01E01.mkv",
        "Lycoris Recoil/Season 01/Lycoris Recoil S01E01.smi",
        "Lycoris Recoil/Season 01/Lycoris Recoil S01E02.mkv",
        "Lycoris Recoil/Season 02/Lycoris Recoil S02E01.mkv",
    ] {
        touch(&root.join(file));
    }
}
