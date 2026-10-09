//! What the integration tests share: the clocks the runner is given, the
//! base every test file's setup builds on (a database, the handles over it, a
//! receive area, and the library of one watch folder), and the list of the
//! files under a folder.

use crate::Handles;
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

use tempfile::TempDir;
use trss_collect::store::{
    channels::ChannelStore, history::HistoryStore, revisions::RevisionStore,
};
use trss_core::{settings::SettingsStore, Clock, Db, DbError, Millis};
use trss_jobs::{
    area::ReceiveArea,
    todo::{self, TodoError, TodoList},
    Follow, JobViews, PlaceStore, Runner,
};
use trss_library::store::{artwork::ArtworkStore, library::LibraryStore};
use trss_subtitles::Sources;

/// A clock that starts at 1_000 and ticks 10 ms at every call, the first call
/// giving the start.
pub fn ticking_clock() -> Clock {
    ticking_from(1_000)
}

/// [`ticking_clock`] starting at `start`.
pub fn ticking_from(start: Millis) -> Clock {
    counting_clock(&Arc::new(AtomicI64::new(start)))
}

/// A clock that ticks 10 ms at every call over `now`, which the caller holds
/// and may advance.
pub fn counting_clock(now: &Arc<AtomicI64>) -> Clock {
    let now = now.clone();
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

/// A clock that shows `at`.
pub fn fixed_clock(at: Millis) -> Clock {
    Arc::new(move || at)
}

/// The nine stores `trss_jobs::todo::list` reads, all over one database, and
/// the address of a cover as `cover/<work>/<image>`.
pub struct TodoStores {
    jobs: JobViews,
    place: PlaceStore,
    follow: Follow,
    library: LibraryStore,
    artwork: ArtworkStore,
    revisions: RevisionStore,
    history: HistoryStore,
    channels: ChannelStore,
    settings: SettingsStore,
}

impl TodoStores {
    pub fn new(db: &Db) -> TodoStores {
        TodoStores {
            jobs: JobViews::new(db.clone()),
            place: PlaceStore::new(db.clone()),
            follow: Follow::new(db.clone()),
            library: LibraryStore::new(db.clone()),
            artwork: ArtworkStore::new(db.clone()),
            revisions: RevisionStore::new(db.clone()),
            history: HistoryStore::new(db.clone()),
            channels: ChannelStore::new(db.clone()),
            settings: SettingsStore::new(db.clone()),
        }
    }

    /// The stores as the to-dos' sources.
    pub fn sources(&self) -> todo::Sources<'_> {
        todo::Sources {
            jobs: &self.jobs,
            place: &self.place,
            follow: &self.follow,
            library: &self.library,
            artwork: &self.artwork,
            revisions: &self.revisions,
            history: &self.history,
            channels: &self.channels,
            settings: &self.settings,
            cover_url: |work, image| format!("cover/{work}/{image}"),
        }
    }

    /// The to-dos that need a person, as the web answers them.
    pub async fn list(&self) -> Result<TodoList, TodoError> {
        todo::list(&self.sources()).await
    }
}

/// The work every library holds, the creator its source has, and its source.
const WORK: &str = "w1";
const CREATOR: &str = "제작자";

/// A tempdir holding the database `app.db`, the handles over it and the
/// receive area. A file's own `Setup` keeps the parts it reads under their
/// names.
pub struct Base {
    pub dir: TempDir,
    pub db: Db,
    pub store: Handles,
    pub area: ReceiveArea,
}

impl Base {
    pub async fn new() -> Base {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let store = Handles::new(db.clone());
        let area = ReceiveArea::in_app_data(dir.path());
        Base {
            dir,
            db,
            store,
            area,
        }
    }

    /// A runner over `sources` that ticks from [`ticking_clock`].
    pub fn runner(&self, sources: Sources) -> Runner {
        self.runner_with(sources, ticking_clock())
    }

    /// A runner over `sources` that is given `clock`.
    pub fn runner_with(&self, sources: Sources, clock: Clock) -> Runner {
        Runner::new(self.store.run.clone(), sources, self.area.clone(), clock)
    }

    /// The to-dos that need a person over this database.
    pub async fn todo(&self) -> TodoList {
        TodoStores::new(&self.db).list().await.unwrap()
    }

    /// The library: the watch folder `f1` at `shows` of the tempdir, whose
    /// work `Show` (`w1`) is described by `shows`. The videos are files of
    /// the bytes `video`, and rows of `media_files`.
    pub async fn library(&self, shows: &Shows) {
        let root = self.dir.path().join("shows");
        let season_dir = root
            .join("Show")
            .join(format!("Season {:02}", shows.season));
        std::fs::create_dir_all(&season_dir).unwrap();
        if shows.apart.is_some() {
            std::fs::create_dir_all(season_dir.join("Apart")).unwrap();
        }
        let season = shows.season;
        let mut sql = format!(
            "INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('{WORK}', 'f1', 'Show');
             INSERT INTO seasons (work_id, number) VALUES ('{WORK}', {season});"
        );
        if shows.source {
            sql.push_str(&format!(
                "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('src', 7, '{CREATOR}', 0);"
            ));
        }
        for &episode in &shows.episodes {
            let path = shows.video(episode);
            std::fs::write(root.join("Show").join(&path), b"video").unwrap();
            sql.push_str(&format!(
                "INSERT INTO episodes (work_id, season, episode)
                     VALUES ('{WORK}', {season}, '{episode:02}');
                 INSERT INTO media_files (work_id, path, season, episode, kind)
                     VALUES ('{WORK}', '{path}', {season}, '{episode:02}', 'video');"
            ));
        }
        let root = root.to_string_lossy().into_owned();
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', ?1, 0)",
                    [root],
                )?;
                c.execute_batch(&sql).map_err(DbError::from)
            })
            .await
            .unwrap();
    }
}

/// The shape of a library: the work `Show` with one season whose episodes
/// have a video each, `Season NN/Show SNNENN.mkv`.
pub struct Shows {
    pub season: u32,
    /// The episodes that have a video.
    pub episodes: Vec<u32>,
    /// Whether the creator `제작자` has the subtitle source `src` (anime 7).
    pub source: bool,
    /// The episode whose video is in a folder of its own, `Apart`.
    pub apart: Option<u32>,
}

impl Default for Shows {
    /// Season 1 with no video and no source.
    fn default() -> Shows {
        Shows {
            season: 1,
            episodes: Vec::new(),
            source: false,
            apart: None,
        }
    }
}

impl Shows {
    /// The video of the episode, from the work's folder.
    fn video(&self, episode: u32) -> String {
        let season = self.season;
        match self.apart == Some(episode) {
            true => format!("Season {season:02}/Apart/Show S{season:02}E{episode:02}.mkv"),
            false => format!("Season {season:02}/Show S{season:02}E{episode:02}.mkv"),
        }
    }
}

/// Every file under `dir`, as paths relative to it, sorted.
pub fn tree(dir: &std::path::Path) -> Vec<String> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten() {
            let path = entry.unwrap().path();
            match path.is_dir() {
                true => walk(root, &path, out),
                false => out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                ),
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}
