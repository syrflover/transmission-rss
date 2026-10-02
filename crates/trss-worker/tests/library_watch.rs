//! Watch folders and the discovery of works, end to end (ticket 0012): the real
//! web API (adding, listing and unregistering a folder, the `다시 확인`
//! command), the real worker (its cycle rescans every folder; commands run
//! under its lock; the archive move keeps works), real temporary folders, a
//! fake Transmission, and no mocks of the discovery itself.
//!
//! The worker's clock is the harness's manual one, so what time a file gets is
//! exact. File system times are set to values that must never be read.

mod common;

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_anilist::{Entry, FuzzyDate};
use trss_collect::{
    commands::rule_archive::work_folder::MovePolicy,
    store::channels::{ChannelInput, ChannelWithRules, RuleInput},
};
use trss_core::settings::SettingsStore;
use trss_library::{
    discovery,
    store::{
        library::{LibraryStore, WatchFolder, WorkRecord},
        seasons::SeasonStore,
    },
};
use trss_worker::{CommandsOutcome, CycleReport, TickOutcome, Worker};

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "x").unwrap();
}

/// Gives every directory under `root` (and `root`) a modification time in 2001,
/// as a folder nobody has touched for a long time has. The periodic scan does
/// not trust a directory changed moments before it listed it, so the trees
/// these tests build must look settled before it can skip them.
fn age_dirs(root: &Path) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() && !path.is_symlink() {
            age_dirs(&path);
        }
    }
    fs::File::open(root)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
        .unwrap();
}

fn text(path: impl AsRef<Path>) -> String {
    path.as_ref().to_str().unwrap().to_owned()
}

/// Sets a file's modification time to the year 2001, which no record may show.
fn age(path: &Path) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
        .unwrap();
}

struct Lib {
    h: Harness,
    /// The one worker of the test, as a process has one: what it remembers
    /// between scans (see `worker::watch`) lives as long as it does.
    worker: Worker,
    api: WebApi,
    library: LibraryStore,
    /// A folder under the harness's temporary directory that watch folders go in.
    media: PathBuf,
}

impl Lib {
    async fn new() -> Lib {
        let h = Harness::without_collect_folder().await;
        let media = h.dir.path().join("media");
        fs::create_dir_all(&media).unwrap();
        Lib {
            api: h.web_api(),
            library: LibraryStore::new(h.db.clone()),
            worker: h.worker().with_move_policy(MovePolicy {
                poll: Duration::from_millis(10),
                timeout: Duration::from_secs(5),
            }),
            h,
            media,
        }
    }

    /// Saves the collection settings through the web API at `version`.
    async fn save_collection(
        &self,
        version: i64,
        collect: &Path,
        archive: Option<&Path>,
    ) -> (StatusCode, Value) {
        let (status, text, body) = self
            .api
            .call(
                "PUT",
                "/api/settings/collection",
                Some(json!({
                    "version": version,
                    "folder": text(collect),
                    "archive_folder": archive.map(text),
                })),
            )
            .await;
        assert!(body != Value::Null, "{text}");
        (status, body)
    }

    async fn unregister(&self, folder: &WatchFolder) -> (StatusCode, Value) {
        let (status, _, body) = self
            .api
            .call(
                "DELETE",
                &format!("/api/library/watch-folders/{}", folder.id),
                None,
            )
            .await;
        (status, body)
    }

    /// The watch folder registered at `path`.
    async fn folder_at(&self, path: &Path) -> Option<WatchFolder> {
        self.library
            .folders()
            .await
            .unwrap()
            .into_iter()
            .find(|f| f.path == text(path))
    }

    fn folder(&self, name: &str) -> PathBuf {
        let path = self.media.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    async fn add(&self, path: &str) -> (StatusCode, Value) {
        let (status, text, body) = self
            .api
            .call(
                "POST",
                "/api/library/watch-folders",
                Some(json!({ "path": path })),
            )
            .await;
        assert!(body != Value::Null, "{text}");
        (status, body)
    }

    /// Adds a folder that must be accepted and returns it as stored.
    async fn register(&self, path: &Path) -> WatchFolder {
        let (status, body) = self.add(&text(path)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        self.library
            .folder(body["folder"]["id"].as_str().unwrap())
            .await
            .unwrap()
            .unwrap()
    }

    async fn list(&self) -> Vec<Value> {
        let (status, text, body) = self
            .api
            .call("GET", "/api/library/watch-folders", None)
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        body["folders"].as_array().unwrap().clone()
    }

    async fn works(&self, folder: &WatchFolder) -> Vec<WorkRecord> {
        self.library.works(&folder.id).await.unwrap()
    }

    async fn work(&self, folder: &WatchFolder, name: &str) -> WorkRecord {
        self.works(folder)
            .await
            .into_iter()
            .find(|w| w.dir_name == name)
            .unwrap_or_else(|| panic!("no work {name}"))
    }

    /// One worker cycle (which rescans every watch folder after the RSS work).
    async fn tick(&self) -> CycleReport {
        match self.worker.tick(&CancellationToken::new()).await.unwrap() {
            TickOutcome::Ran(report) => report,
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    fn worker(&self) -> Worker {
        self.worker.clone()
    }

    async fn send(&self, id: &str, kind: &str, payload: Value) -> (StatusCode, Value) {
        let (status, _, body) = self
            .api
            .call(
                "POST",
                "/api/commands",
                Some(json!({ "id": id, "kind": kind, "payload": payload })),
            )
            .await;
        (status, body)
    }

    async fn run_commands(&self) -> CommandsOutcome {
        self.worker()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap()
    }

    async fn command(&self, id: &str) -> Value {
        let (status, text, body) = self
            .api
            .call("GET", &format!("/api/commands/{id}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        body
    }
}

fn lycoris(root: &Path) {
    for file in [
        "Lycoris Recoil/Season 01/Lycoris Recoil S01E01.mkv",
        "Lycoris Recoil/Season 01/Lycoris Recoil S01E01.smi",
        "Lycoris Recoil/Season 01/Lycoris Recoil S01E02.mkv",
        "Lycoris Recoil/Season 02/Lycoris Recoil S02E01.mkv",
    ] {
        touch(&root.join(file));
    }
}

// --- registering a folder ------------------------------------------------------------

#[tokio::test]
async fn registering_a_folder_records_the_work_its_seasons_and_episodes_and_says_how_many_works() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);

    let (status, body) = lib.add(&format!("{}/", root.display())).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["works_found"], 1);
    assert_eq!(body["folder"]["path"], text(&root));
    assert_eq!(body["folder"]["works"], 1);
    assert_eq!(body["folder"]["linked_works"], 0);
    assert_eq!(body["folder"]["new_works"], 0);
    assert!(body["folder"]["checked_at"].as_i64().unwrap() > 0);
    assert_eq!(body["folder"]["error"], Value::Null);

    let folder = lib.library.folders().await.unwrap().remove(0);
    let work = lib.work(&folder, "Lycoris Recoil").await;
    assert_eq!(work.seasons, [1, 2]);
    let shape: Vec<(u32, &str, Vec<&str>)> = work
        .episodes
        .iter()
        .map(|e| {
            (
                e.season,
                e.episode.as_str(),
                e.files.iter().map(|f| f.path.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            (
                1,
                "01",
                vec![
                    "Season 01/Lycoris Recoil S01E01.mkv",
                    "Season 01/Lycoris Recoil S01E01.smi"
                ]
            ),
            (1, "02", vec!["Season 01/Lycoris Recoil S01E02.mkv"]),
            (2, "01", vec!["Season 02/Lycoris Recoil S02E01.mkv"]),
        ]
    );
    assert!(work.unrecognized.is_empty());
    // Everything there at the first check is of unknown age.
    assert_eq!(work.first_seen_at, None);
    assert!(work.files().values().all(|f| f.added_at.is_none()));

    let listed = lib.list().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["works"], 1);
    // Reading a folder writes nothing into it.
    assert!(!root.join("Lycoris Recoil/.trss").exists());
}

#[tokio::test]
async fn folders_that_overlap_a_registered_one_or_cannot_be_read_are_refused_with_a_reason() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    touch(&root.join("Show/Season 01/Show S01E01.mkv"));
    lib.register(&root).await;

    let refused = |body: &Value| body["message"].as_str().unwrap().to_owned();

    // The same folder, also through a link and with a trailing slash.
    let link = lib.media.join("anime-link");
    std::os::unix::fs::symlink(&root, &link).unwrap();
    for path in [text(&root), format!("{}/", root.display()), text(&link)] {
        let (status, body) = lib.add(&path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert!(refused(&body).contains("이미 등록한"), "{body}");
    }

    // Inside a registered folder.
    let (status, body) = lib.add(&text(root.join("Show"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(refused(&body).contains("안에 있는 폴더"), "{body}");

    // Around a registered folder.
    let (status, body) = lib.add(&text(&lib.media)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        refused(&body).contains("안에 이미 등록한 감시 폴더"),
        "{body}"
    );

    // Missing, not a folder, not absolute, empty.
    let file = lib.media.join("file.txt");
    touch(&file);
    let (status, body) = lib.add(&text(lib.media.join("nope"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(refused(&body).contains("찾지 못했어요"), "{body}");
    let (_, body) = lib.add(&text(&file)).await;
    assert!(refused(&body).contains("폴더가 아니에요"), "{body}");
    let (_, body) = lib.add("relative/anime").await;
    assert!(refused(&body).contains("전체 경로"), "{body}");
    let (_, body) = lib.add("  ").await;
    assert!(refused(&body).contains("입력해 주세요"), "{body}");

    // A folder the web cannot read.
    let locked = lib.folder("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let (status, body) = lib.add(&text(&locked)).await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(refused(&body).contains("권한"), "{body}");

    // Nothing but the first folder was registered.
    assert_eq!(lib.library.folders().await.unwrap().len(), 1);
}

#[tokio::test]
async fn files_that_do_not_fit_are_counted_on_the_work_and_hidden_ones_are_not_discovered() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    for file in [
        "Show/Season 01/Show S01E01.mkv",
        // The season in the name is not the folder's.
        "Show/Season 01/Show S03E01.mkv",
        // Outside a season folder.
        "Show/Show S01E05.mkv",
        // In a folder inside a season folder.
        "Show/Season 01/[Batch] Show/Show 02.mkv",
        // Still downloading.
        "Show/Season 01/Show S01E04.mkv.part",
        // Hidden: never counted.
        "Show/.trss/subs/Show S01E01.ass",
        "Show/Season 01/.trss/cache.mkv",
        ".hidden/Season 01/H S01E01.mkv",
    ] {
        touch(&root.join(file));
    }
    lib.register(&root).await;
    let folder = lib.library.folders().await.unwrap().remove(0);
    assert_eq!(lib.works(&folder).await.len(), 1);

    let show = lib.work(&folder, "Show").await;
    assert_eq!(show.episodes.len(), 1);
    let mut reasons: Vec<(&str, &str)> = show
        .unrecognized
        .iter()
        .map(|u| (u.path.as_str(), u.reason.code()))
        .collect();
    reasons.sort();
    assert_eq!(
        reasons,
        [
            ("Season 01/Show S01E04.mkv.part", "partial"),
            ("Season 01/Show S03E01.mkv", "season_mismatch"),
            ("Season 01/[Batch] Show/Show 02.mkv", "in_subfolder"),
            ("Show S01E05.mkv", "outside_season"),
        ]
    );
}

// --- what a later check makes of a change --------------------------------------------

#[tokio::test]
async fn only_what_appears_after_the_first_check_gets_a_time_and_file_times_change_nothing() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    let folder = lib.register(&root).await;
    let first = lib.work(&folder, "Lycoris Recoil").await;

    // File times, in any direction, are not read.
    age(&root.join("Lycoris Recoil/Season 01/Lycoris Recoil S01E01.mkv"));
    lib.h.advance(60_000);
    let t1 = lib.h.now();
    touch(&root.join("Lycoris Recoil/Season 01/Lycoris Recoil S01E03.mkv"));
    touch(&root.join("Brand New/Season 01/Brand New S01E01.mkv"));
    age(&root.join("Brand New/Season 01/Brand New S01E01.mkv"));
    lib.tick().await;

    let work = lib.work(&folder, "Lycoris Recoil").await;
    assert_eq!(work.id, first.id);
    let files = work.files();
    assert_eq!(
        files["Season 01/Lycoris Recoil S01E03.mkv"].added_at,
        Some(t1)
    );
    for old in [
        "Season 01/Lycoris Recoil S01E01.mkv",
        "Season 01/Lycoris Recoil S01E01.smi",
        "Season 01/Lycoris Recoil S01E02.mkv",
        "Season 02/Lycoris Recoil S02E01.mkv",
    ] {
        assert_eq!(files[old].added_at, None, "{old}");
    }
    assert_eq!(work.first_seen_at, None);
    let fresh = lib.work(&folder, "Brand New").await;
    assert_eq!(fresh.first_seen_at, Some(t1));
    assert_eq!(fresh.files().values().next().unwrap().added_at, Some(t1));

    // A third check years later changes no recorded time.
    lib.h.advance(3_600_000);
    touch(&root.join("Lycoris Recoil/Season 01/Lycoris Recoil S01E04.mkv"));
    lib.tick().await;
    let again = lib.work(&folder, "Lycoris Recoil").await;
    let files = again.files();
    assert_eq!(
        files["Season 01/Lycoris Recoil S01E03.mkv"].added_at,
        Some(t1)
    );
    assert_eq!(
        files["Season 01/Lycoris Recoil S01E04.mkv"].added_at,
        Some(lib.h.now())
    );
    assert_eq!(lib.work(&folder, "Brand New").await.first_seen_at, Some(t1));
}

#[tokio::test]
async fn the_list_counts_new_works_found_within_a_week_after_the_first_check() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    // The worker's clock is the real time here, as the web's is.
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    lib.h.clock.store(now, std::sync::atomic::Ordering::SeqCst);
    lib.register(&root).await;
    assert_eq!(lib.list().await[0]["new_works"], 0);

    touch(&root.join("Brand New/Season 01/Brand New S01E01.mkv"));
    lib.tick().await;
    let row = &lib.list().await[0];
    assert_eq!(
        (row["works"].as_u64(), row["new_works"].as_u64()),
        (Some(2), Some(1))
    );
}

#[tokio::test]
async fn a_deleted_episode_file_drops_out_and_a_deleted_work_folder_keeps_its_id() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    touch(&root.join("Other/Season 01/Other S01E01.mkv"));
    let folder = lib.register(&root).await;
    let before = lib.work(&folder, "Lycoris Recoil").await;
    let other = lib.work(&folder, "Other").await;

    fs::remove_file(root.join("Lycoris Recoil/Season 01/Lycoris Recoil S01E02.mkv")).unwrap();
    fs::remove_dir_all(root.join("Other")).unwrap();
    lib.h.advance(1000);
    lib.tick().await;

    let after = lib.work(&folder, "Lycoris Recoil").await;
    assert_eq!(after.id, before.id);
    assert!(!after.missing);
    assert!(after.episodes.iter().all(|e| e.episode != "02"));
    assert_eq!(after.episodes.len(), before.episodes.len() - 1);

    let gone = lib.work(&folder, "Other").await;
    assert_eq!(gone.id, other.id);
    assert!(gone.missing);
    let row = &lib.list().await[0];
    assert_eq!(
        (row["works"].as_u64(), row["missing_works"].as_u64()),
        (Some(2), Some(1))
    );

    // It comes back: the same work, no longer missing.
    touch(&root.join("Other/Season 01/Other S01E01.mkv"));
    lib.h.advance(1000);
    lib.tick().await;
    let back = lib.work(&folder, "Other").await;
    assert_eq!(back.id, other.id);
    assert!(!back.missing);
}

// --- a folder that cannot be read ----------------------------------------------------

#[tokio::test]
async fn an_unreadable_watch_folder_shows_its_reason_and_stops_neither_the_others_nor_rss() {
    let lib = Lib::new().await;
    let (a, b) = (lib.folder("a"), lib.folder("b"));
    touch(&a.join("Show A/Season 01/Show A S01E01.mkv"));
    touch(&b.join("Show B/Season 01/Show B S01E01.mkv"));
    let fa = lib.register(&a).await;
    let fb = lib.register(&b).await;
    let before = lib.works(&fa).await;

    // An RSS channel that the cycle must still run.
    SettingsStore::new(lib.h.db.clone())
        .put_collection(0, text(lib.folder("collect")), None)
        .await
        .unwrap();
    lib.h
        .channels
        .create_channel_with_rules(ChannelInput::new(lib.h.feeds.url("feed-a")), feed_a_rules())
        .await
        .unwrap();

    fs::set_permissions(&a, fs::Permissions::from_mode(0o000)).unwrap();
    touch(&b.join("Show B/Season 01/Show B S01E02.mkv"));
    lib.h.advance(1000);
    let report = lib.tick().await;
    fs::set_permissions(&a, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(report.channels_read, 1);
    assert!(report.items_seen > 0, "{report:?}");

    let rows = lib.list().await;
    assert!(
        rows[0]["error"].as_str().unwrap().contains("권한"),
        "{rows:?}"
    );
    assert_eq!(rows[0]["works"], 1);
    assert_eq!(rows[1]["error"], Value::Null);
    // The earlier records of the folder that could not be read are as they were.
    assert_eq!(lib.works(&fa).await, before);
    // The other folder was read.
    let b_work = lib.work(&fb, "Show B").await;
    assert!(b_work.files().contains_key("Season 01/Show B S01E02.mkv"));

    // Readable again: the error is gone.
    lib.h.advance(1000);
    lib.tick().await;
    assert_eq!(lib.list().await[0]["error"], Value::Null);
}

// --- unregistering -------------------------------------------------------------------

/// The automatic jobs waiting in the database: cover and season searches.
async fn queued_jobs(db: &trss_core::Db) -> i64 {
    db.run::<_, trss_core::DbError, _>(|c| {
        Ok(c.query_row(
            "SELECT (SELECT count(*) FROM work_artwork WHERE job IS NOT NULL)
                  + (SELECT count(*) FROM season_info WHERE job IS NOT NULL)",
            [],
            |r| r.get(0),
        )?)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn unregistering_takes_the_works_out_and_the_same_path_brings_them_back_as_they_were() {
    let lib = Lib::new().await;
    let (a, b) = (lib.folder("a"), lib.folder("b"));
    lycoris(&a);
    touch(&b.join("Keep/Season 01/Keep S01E01.mkv"));
    let fa = lib.register(&a).await;
    let fb = lib.register(&b).await;
    let on_disk = |root: &Path| {
        let mut out = Vec::new();
        fn walk(dir: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                }
                out.push(p.display().to_string());
            }
        }
        walk(root, &mut out);
        out.sort();
        out
    };
    let files_before = on_disk(&a);

    // What the user chose for the work: no cover, and season 1 linked.
    let work = lib.work(&fa, "Lycoris Recoil").await;
    let (status, _, cover) = lib
        .api
        .call(
            "GET",
            &format!("/api/library/works/{}/artwork", work.id),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, cleared) = lib
        .api
        .call(
            "POST",
            &format!("/api/library/works/{}/artwork/clear", work.id),
            Some(json!({ "version": cover["version"] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    let seasons = SeasonStore::new(lib.h.db.clone());
    seasons
        .put_entry(Entry {
            id: 143270,
            romaji: Some("Lycoris Recoil".into()),
            english: None,
            native: None,
            format: Some("TV".into()),
            status: Some("FINISHED".into()),
            episodes: Some(13),
            start: FuzzyDate::default(),
            end: FuzzyDate::default(),
            studios: Vec::new(),
            genres: Vec::new(),
            description: None,
            airing: Vec::new(),
            sequels: Vec::new(),
            fetched_at: 1,
        })
        .await
        .unwrap();
    let link = seasons.link(&work.id, 1).await.unwrap();
    let linked = seasons
        .set_links(&work.id, 1, link.version, vec![143270])
        .await
        .unwrap();
    // Only Keep's cover and first season searches are waiting.
    assert_eq!(queued_jobs(&lib.h.db).await, 2);

    let (status, body) = lib.unregister(&fa).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["removed_works"], 1);

    // Out of the library: the folder list, the work list, the work's screens.
    assert_eq!(lib.list().await.len(), 1);
    assert!(lib.folder_at(&a).await.is_none());
    let (_, _, works) = lib.api.call("GET", "/api/library/works", None).await;
    let names: Vec<&str> = works["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Keep"]);
    for uri in [
        format!("/api/library/works/{}", work.id),
        format!("/api/library/works/{}/artwork", work.id),
    ] {
        let (status, _, _) = lib.api.call("GET", &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
    assert_eq!(lib.works(&fb).await.len(), 1);
    assert_eq!(on_disk(&a), files_before);
    // A cycle does not read it.
    lib.tick().await;
    assert_eq!(
        lib.library.works(&fa.id).await.unwrap(),
        std::slice::from_ref(&work)
    );

    let (status, body) = lib.unregister(&fa).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // The same path again: the same folder and work, with what was chosen,
    // and no new search.
    let again = lib.register(&a).await;
    assert_eq!(again.id, fa.id);
    assert_eq!(lib.work(&again, "Lycoris Recoil").await.id, work.id);
    let (status, _, cover) = lib
        .api
        .call(
            "GET",
            &format!("/api/library/works/{}/artwork", work.id),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cover, cleared);
    assert_eq!(seasons.link(&work.id, 1).await.unwrap(), linked);
    assert_eq!(queued_jobs(&lib.h.db).await, 2);
    assert_eq!(on_disk(&a), files_before);
}

#[tokio::test]
async fn the_periodic_scan_skips_unchanged_directories_and_rescan_reads_everything() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    let folder = lib.register(&root).await;
    age_dirs(&root);
    let season = root.join("Lycoris Recoil/Season 01");
    let work = || lib.work(&folder, "Lycoris Recoil");

    // The first scan after the worker starts reads everything and remembers it.
    lib.h.advance(1000);
    lib.tick().await;
    assert_eq!(work().await.files().len(), 4);

    // A change that leaves the season folder's modification time as it was (a
    // coarse clock) is not seen by the periodic scan...
    touch(&season.join("Lycoris Recoil S01E03.mkv"));
    age_dirs(&season);
    lib.h.advance(1000);
    lib.tick().await;
    assert!(!work()
        .await
        .files()
        .contains_key("Season 01/Lycoris Recoil S01E03.mkv"));

    // ...but `다시 확인` reads every directory, and the file gets that time.
    lib.h.advance(1000);
    let (status, body) = lib
        .send(
            "rescan-full",
            "watch_rescan",
            json!({ "folder_id": folder.id }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(lib.run_commands().await, CommandsOutcome::Ran(1));
    assert_eq!(
        work().await.files()["Season 01/Lycoris Recoil S01E03.mkv"].added_at,
        Some(lib.h.now())
    );

    // An ordinary change (the folder's time moves) is found by the next cycle,
    // while the folders around it are not listed again.
    touch(&season.join("Lycoris Recoil S01E04.mkv"));
    lib.h.advance(1000);
    lib.tick().await;
    assert_eq!(
        work().await.files()["Season 01/Lycoris Recoil S01E04.mkv"].added_at,
        Some(lib.h.now())
    );
    // The time of a file's modification is not an added time.
    age(&season.join("Lycoris Recoil S01E04.mkv"));
    lib.h.advance(1000);
    lib.tick().await;
    assert_eq!(
        work().await.files()["Season 01/Lycoris Recoil S01E04.mkv"].added_at,
        Some(lib.h.now() - 1000)
    );
}

// --- the collect and archive folders are always watched ---------------------------------

#[tokio::test]
async fn saving_the_collect_and_archive_folders_registers_and_reads_them_and_they_cannot_be_unregistered(
) {
    let lib = Lib::new().await;
    let (collect, archive) = (lib.folder("Shows (current)"), lib.folder("Shows"));
    touch(&collect.join("A/Season 01/A S01E01.mkv"));
    touch(&archive.join("B/Season 01/B S01E01.mkv"));

    let (status, body) = lib.save_collection(0, &collect, Some(&archive)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let listed = lib.list().await;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["path"], text(&collect));
    assert_eq!(listed[1]["path"], text(&archive));
    for folder in &listed {
        assert_eq!(folder["automatic"], true, "{folder}");
        assert_eq!(folder["works"], 1);
        assert!(folder["checked_at"].as_i64().unwrap() > 0);
        assert_eq!(folder["error"], Value::Null);
    }
    // The first reading is the baseline: nothing is dated, nothing is new.
    let fc = lib.folder_at(&collect).await.unwrap();
    assert!(fc.baselined);
    let a = lib.work(&fc, "A").await;
    assert_eq!(a.first_seen_at, None);
    assert!(a.files().values().all(|f| f.added_at.is_none()));
    assert_eq!(listed[0]["new_works"], 0);

    // Neither can be unregistered, and both stay.
    for folder in [&fc, &lib.folder_at(&archive).await.unwrap()] {
        let (status, body) = lib.unregister(folder).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap()
                .contains("수집 폴더나 보관 폴더"),
            "{body}"
        );
    }
    assert_eq!(lib.list().await.len(), 2);

    // A folder added by hand is not automatic.
    let other = lib.folder("other");
    let by_hand = lib.register(&other).await;
    assert!(!by_hand.automatic);
    assert_eq!(lib.unregister(&by_hand).await.0, StatusCode::OK);

    // A later cycle reads them like any watch folder: only new files are dated.
    touch(&collect.join("A/Season 01/A S01E02.mkv"));
    lib.h.advance(1000);
    lib.tick().await;
    let a = lib.work(&fc, "A").await;
    assert_eq!(a.files()["Season 01/A S01E01.mkv"].added_at, None);
    assert_eq!(
        a.files()["Season 01/A S01E02.mkv"].added_at,
        Some(lib.h.clock.load(std::sync::atomic::Ordering::SeqCst))
    );
}

#[tokio::test]
async fn changing_the_collect_folder_swaps_its_watch_folder_and_adopts_one_registered_by_hand() {
    let lib = Lib::new().await;
    let (a, b, c) = (lib.folder("a"), lib.folder("b"), lib.folder("c"));
    touch(&a.join("Old/Season 01/Old S01E01.mkv"));
    touch(&b.join("InB/Season 01/InB S01E01.mkv"));
    touch(&c.join("InC/Season 01/InC S01E01.mkv"));
    let (status, body) = lib.save_collection(0, &a, Some(&b)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let fa = lib.folder_at(&a).await.unwrap();
    let fb = lib.folder_at(&b).await.unwrap();
    let by_hand = lib.register(&c).await;
    let in_c = lib.work(&by_hand, "InC").await;
    assert!(!by_hand.automatic);

    // /c replaces /a: /a is unregistered (its work leaves the library but is
    // kept), /c keeps its records.
    let old_work = lib.work(&fa, "Old").await;
    let (status, body) = lib.save_collection(1, &c, Some(&b)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(lib.folder_at(&a).await.is_none());
    assert_eq!(
        lib.library.works(&fa.id).await.unwrap(),
        std::slice::from_ref(&old_work)
    );
    assert!(lib
        .library
        .overview()
        .await
        .unwrap()
        .iter()
        .all(|w| w.id != old_work.id));
    let adopted = lib.folder_at(&c).await.unwrap();
    assert_eq!(adopted.id, by_hand.id);
    assert!(adopted.automatic);
    assert_eq!(lib.work(&adopted, "InC").await, in_c);
    // The archive folder was not touched.
    let kept = lib.folder_at(&b).await.unwrap();
    assert_eq!(kept.id, fb.id);
    assert_eq!(lib.works(&kept).await.len(), 1);
    // The files of the folder that went are still there.
    assert!(a.join("Old/Season 01/Old S01E01.mkv").exists());
    assert_eq!(lib.list().await.len(), 2);

    // Clearing the archive folder takes its watch folder away.
    let (status, body) = lib.save_collection(2, &c, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(lib.folder_at(&b).await.is_none());
    assert_eq!(lib.list().await.len(), 1);

    // A folder that went can be registered by hand afterwards, and brings its
    // work back.
    let again = lib.register(&a).await;
    assert!(!again.automatic);
    assert_eq!(again.id, fa.id);
    assert_eq!(lib.work(&again, "Old").await.id, old_work.id);
}

#[tokio::test]
async fn a_collect_or_archive_folder_that_overlaps_a_registered_folder_is_not_saved() {
    let lib = Lib::new().await;
    let downloads = lib.folder("downloads");
    let current = lib.folder("downloads/Shows (current)");
    let archive = lib.folder("archive");
    lib.register(&downloads).await;

    let (status, body) = lib.save_collection(0, &current, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("수집 폴더") && message.contains("안에 있어요"),
        "{message}"
    );
    // Nothing was saved or registered.
    let (_, _, settings) = lib.api.call("GET", "/api/settings/collection", None).await;
    assert_eq!(settings["folder"], Value::Null);
    assert_eq!(settings["version"], 0);
    assert_eq!(lib.list().await.len(), 1);

    // A folder around a registered one is refused as well.
    let inner = lib.folder("inner/watched");
    let around = lib.media.join("inner");
    lib.register(&inner).await;
    let (status, body) = lib.save_collection(0, &archive, Some(&around)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("보관 폴더") && message.contains("등록한 감시 폴더"),
        "{message}"
    );
    assert!(lib.folder_at(&archive).await.is_none());
    assert_eq!(lib.list().await.len(), 2);
}

#[tokio::test]
async fn a_collect_folder_that_cannot_be_read_is_still_saved_and_shows_why_on_its_row() {
    use std::os::unix::fs::PermissionsExt;
    let lib = Lib::new().await;
    let collect = lib.folder("locked");
    touch(&collect.join("A/Season 01/A S01E01.mkv"));
    fs::set_permissions(&collect, fs::Permissions::from_mode(0o000)).unwrap();
    let (status, body) = lib.save_collection(0, &collect, None).await;
    // Whether the web could see the folder at all is the settings check's matter
    // (a process allowed everything does not even fail here).
    fs::set_permissions(&collect, fs::Permissions::from_mode(0o755)).unwrap();
    if status != StatusCode::OK {
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        return;
    }
    let folder = lib.folder_at(&collect).await.unwrap();
    assert!(folder.automatic);
    // The worker reads it when it can; until then there is no first reading.
    lib.h.advance(1000);
    lib.tick().await;
    assert_eq!(lib.works(&folder).await.len(), 1);
}

#[tokio::test]
async fn the_worker_registers_the_folders_of_settings_saved_before_they_were_watched() {
    let lib = Lib::new().await;
    let (collect, archive) = (lib.folder("Shows (current)"), lib.folder("Shows"));
    touch(&collect.join("A/Season 01/A S01E01.mkv"));
    touch(&archive.join("B/Season 01/B S01E01.mkv"));
    // As a database from before the folders were watched has them: set, not watched.
    SettingsStore::new(lib.h.db.clone())
        .put_collection(0, text(&collect), Some(text(&archive)))
        .await
        .unwrap();
    // One of them was registered by hand.
    let by_hand = lib.register(&archive).await;
    let in_b = lib.work(&by_hand, "B").await;
    assert!(lib.library.folders().await.unwrap().len() == 1);

    lib.h.advance(1000);
    lib.tick().await;

    let folders = lib.library.folders().await.unwrap();
    assert_eq!(folders.len(), 2);
    assert!(folders.iter().all(|f| f.automatic));
    let adopted = lib.folder_at(&archive).await.unwrap();
    assert_eq!(adopted.id, by_hand.id);
    assert_eq!(lib.work(&adopted, "B").await, in_b);
    // The new one was read in the same cycle, as its baseline.
    let fc = lib.folder_at(&collect).await.unwrap();
    assert!(fc.baselined);
    assert_eq!(lib.work(&fc, "A").await.first_seen_at, None);

    // The next cycle changes none of that.
    lib.h.advance(1000);
    lib.tick().await;
    assert_eq!(lib.library.folders().await.unwrap().len(), 2);
    assert_eq!(lib.folder_at(&collect).await.unwrap().id, fc.id);
}

// --- 다시 확인 ---------------------------------------------------------------------------

#[tokio::test]
async fn rescan_is_accepted_as_a_command_the_worker_runs_once_per_command_id() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    let folder = lib.register(&root).await;
    let checked = lib
        .library
        .folder(&folder.id)
        .await
        .unwrap()
        .unwrap()
        .checked_at;

    touch(&root.join("Brand New/Season 01/Brand New S01E01.mkv"));
    lib.h.advance(5000);
    let payload = json!({ "folder_id": folder.id });
    let (status, body) = lib
        .send("rescan-0001", "watch_rescan", payload.clone())
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["state"], "pending");
    // Accepted is not done: nothing was read yet.
    assert_eq!(lib.works(&folder).await.len(), 1);
    // A second open rescan of the same folder is refused while this one is open.
    let (status, body) = lib
        .send("rescan-0002", "watch_rescan", payload.clone())
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    // The same command again is the same command.
    let (status, _) = lib
        .send("rescan-0001", "watch_rescan", payload.clone())
        .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(lib.run_commands().await, CommandsOutcome::Ran(1));
    let command = lib.command("rescan-0001").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "scanned");
    let row = &lib.list().await[0];
    assert_eq!(row["works"], 2);
    assert_eq!(row["checked_at"], lib.h.now());
    assert_ne!(Some(lib.h.now()), checked);

    // The same ID sent again does not read the folder again.
    touch(&root.join("Another/Season 01/Another S01E01.mkv"));
    lib.h.advance(5000);
    let (status, body) = lib
        .send("rescan-0001", "watch_rescan", payload.clone())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "done");
    assert_eq!(lib.run_commands().await, CommandsOutcome::Idle);
    assert_eq!(lib.list().await[0]["works"], 2);

    // The same ID with another folder is a mismatch.
    let (status, _) = lib
        .send(
            "rescan-0001",
            "watch_rescan",
            json!({ "folder_id": "other" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A folder that is not registered is refused, and a bad payload too.
    let (status, _) = lib
        .send(
            "rescan-0003",
            "watch_rescan",
            json!({ "folder_id": "nope" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = lib.send("rescan-0004", "watch_rescan", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A new ID is a new action: the folder is read again.
    let (status, _) = lib.send("rescan-0005", "watch_rescan", payload).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(lib.run_commands().await, CommandsOutcome::Ran(1));
    assert_eq!(lib.list().await[0]["works"], 3);
}

#[tokio::test]
async fn a_rescan_of_an_unreadable_folder_ends_failed_with_the_reason_and_keeps_the_records() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    let folder = lib.register(&root).await;
    let before = lib.works(&folder).await;

    fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();
    lib.send(
        "rescan-0001",
        "watch_rescan",
        json!({ "folder_id": folder.id }),
    )
    .await;
    lib.run_commands().await;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();

    let command = lib.command("rescan-0001").await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(command["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("권한"));
    assert_eq!(lib.works(&folder).await, before);
    assert!(lib.list().await[0]["error"]
        .as_str()
        .unwrap()
        .contains("권한"));
}

// --- the archive move keeps the work ---------------------------------------------------

struct Archive {
    lib: Lib,
    collect: PathBuf,
    archive: PathBuf,
    collect_folder: WatchFolder,
    archive_folder: WatchFolder,
    channel: ChannelWithRules,
}

impl Archive {
    async fn new() -> Archive {
        let lib = Lib::new().await;
        let collect = lib.folder("Shows (current)");
        let archive = lib.folder("Shows");
        lib.h.feeds.set_xml(
            "feed-a",
            r#"<?xml version="1.0"?><rss version="2.0"><channel><title>x</title>
            <link>http://x/</link><description>d</description></channel></rss>"#,
        );
        let channel = lib
            .h
            .channels
            .create_channel_with_rules(
                ChannelInput::new(lib.h.feeds.url("feed-a")),
                vec![
                    RuleInput {
                        r#match: Some("Clevatess".into()),
                        directory: "Clevatess/Season 02".into(),
                        ..RuleInput::default()
                    },
                    RuleInput {
                        r#match: Some("Solo".into()),
                        directory: "Solo/Season 01".into(),
                        ..RuleInput::default()
                    },
                ],
            )
            .await
            .unwrap();
        touch(&collect.join("Clevatess/Season 02/Clevatess S02E01.mkv"));
        touch(&collect.join("Clevatess/Season 02/Clevatess S02E02.mkv"));
        touch(&collect.join("Solo/Season 01/Solo S01E01.mkv"));
        // The archive already has the earlier season of Clevatess.
        touch(&archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"));
        // Saving the settings makes both folders watch folders and reads them.
        let (status, body) = lib.save_collection(0, &collect, Some(&archive)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let collect_folder = lib.folder_at(&collect).await.unwrap();
        let archive_folder = lib.folder_at(&archive).await.unwrap();
        assert!(collect_folder.automatic && archive_folder.automatic);
        Archive {
            lib,
            collect,
            archive,
            collect_folder,
            archive_folder,
            channel,
        }
    }

    async fn archive_rule(&self, id: &str, rule: usize) -> Value {
        let (status, body) = self
            .lib
            .send(
                id,
                "rule_archive",
                json!({ "rule_id": self.channel.rules[rule].id, "direction": "archive" }),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(self.lib.run_commands().await, CommandsOutcome::Ran(1));
        let command = self.lib.command(id).await;
        assert_eq!(command["state"], "done", "{command}");
        command
    }
}

#[tokio::test]
async fn archiving_a_work_keeps_its_id_under_the_archive_folder() {
    let a = Archive::new().await;
    let solo = a.lib.work(&a.collect_folder, "Solo").await;
    assert_eq!(solo.episodes.len(), 1);

    a.archive_rule("archive-solo-1", 1).await;
    assert!(!a.collect.join("Solo").exists());
    assert!(a.archive.join("Solo/Season 01/Solo S01E01.mkv").exists());

    // Same ID, now under the archive folder, with nothing left behind.
    assert!(a
        .lib
        .works(&a.collect_folder)
        .await
        .iter()
        .all(|w| w.dir_name != "Solo"));
    let moved = a.lib.work(&a.archive_folder, "Solo").await;
    assert_eq!(moved.id, solo.id);
    assert_eq!(moved.episodes, solo.episodes);
    assert_eq!(moved.first_seen_at, solo.first_seen_at);

    // The next cycle reads the folders as they are, and the work is the same one.
    a.lib.h.advance(1000);
    a.lib.tick().await;
    let read = a.lib.work(&a.archive_folder, "Solo").await;
    assert_eq!(read.id, solo.id);
    assert!(!read.missing);
    assert!(a
        .lib
        .works(&a.collect_folder)
        .await
        .iter()
        .all(|w| w.dir_name != "Solo"));
}

#[tokio::test]
async fn archiving_into_an_existing_work_folder_merges_into_the_destinations_id() {
    let a = Archive::new().await;
    let moved = a.lib.work(&a.collect_folder, "Clevatess").await;
    let kept = a.lib.work(&a.archive_folder, "Clevatess").await;
    assert_ne!(moved.id, kept.id);

    a.archive_rule("archive-clev-1", 0).await;

    assert!(a
        .lib
        .works(&a.collect_folder)
        .await
        .iter()
        .all(|w| w.dir_name != "Clevatess"));
    let merged = a.lib.work(&a.archive_folder, "Clevatess").await;
    assert_eq!(merged.id, kept.id);
    assert_eq!(merged.seasons, [1, 2]);
    assert_eq!(merged.episodes.len(), 3);

    // One row, not two, and the same after the next reading.
    a.lib.h.advance(1000);
    a.lib.tick().await;
    let read = a.lib.work(&a.archive_folder, "Clevatess").await;
    assert_eq!(read.id, kept.id);
    assert_eq!(read.episodes.len(), 3);
    let rows: Vec<_> = a
        .lib
        .works(&a.archive_folder)
        .await
        .into_iter()
        .filter(|w| w.dir_name == "Clevatess")
        .collect();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn a_folder_moved_by_hand_is_a_new_work_and_the_old_one_is_missing() {
    let a = Archive::new().await;
    let solo = a.lib.work(&a.collect_folder, "Solo").await;

    fs::rename(a.collect.join("Solo"), a.archive.join("Solo")).unwrap();
    a.lib.h.advance(1000);
    a.lib.tick().await;

    let old = a.lib.work(&a.collect_folder, "Solo").await;
    assert_eq!(old.id, solo.id);
    assert!(old.missing);
    let new = a.lib.work(&a.archive_folder, "Solo").await;
    assert_ne!(new.id, solo.id);
    assert_eq!(new.first_seen_at, Some(a.lib.h.now()));
}

// --- a large library -----------------------------------------------------------------

// --- the library list ----------------------------------------------------------------

impl Lib {
    /// Every work, by title, read page by page as the screen does.
    async fn library_list(&self) -> Vec<Value> {
        let mut works = Vec::new();
        let mut after = String::new();
        loop {
            let uri = format!("/api/library/works?sort=title&limit=200&after={after}");
            let (status, text, body) = self.api.call("GET", &uri, None).await;
            assert_eq!(status, StatusCode::OK, "{text}");
            works.extend(body["items"].as_array().unwrap().iter().cloned());
            match body["next"].as_str() {
                Some(next) => after = next.to_owned(),
                None => return works,
            }
        }
    }
}

#[tokio::test]
async fn the_library_list_answers_ranges_flags_and_times_of_every_work() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    // Season 2 is the latest: videos 1-5, subtitles 1-2 and 4-5.
    for e in 1..=5 {
        touch(&root.join(format!("Alpha/Season 02/Alpha S02E{e:02}.mkv")));
        if e != 3 {
            touch(&root.join(format!("Alpha/Season 02/Alpha S02E{e:02}.ko.ass")));
        }
    }
    touch(&root.join("Alpha/Season 01/Alpha S01E01.mkv"));
    // A subtitle whose episode cannot be read.
    touch(&root.join("Beta/Season 01/Beta S01E01.mkv"));
    touch(&root.join("Beta/Season 01/Beta-extras.srt"));
    touch(&root.join("Gamma/Season 01/Gamma S01E13.mkv"));
    touch(&root.join("Gamma/Season 01/Gamma S01E13.ass"));
    let folder = lib.register(&root).await;

    // After the first check a new subtitle and a new work get the worker's time.
    touch(&root.join("Alpha/Season 02/Alpha S02E03.ko.ass"));
    touch(&root.join("Delta/Season 01/Delta S01E01.mkv"));
    lib.h.advance(5_000);
    lib.tick().await;
    let later = lib.h.now();

    // A work folder that goes away stays in the list.
    fs::remove_dir_all(root.join("Gamma")).unwrap();
    lib.h.advance(5_000);
    lib.tick().await;

    let list = lib.library_list().await;
    let names: Vec<&str> = list.iter().map(|w| w["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Alpha", "Beta", "Delta", "Gamma"]);
    let by = |name: &str| list.iter().find(|w| w["name"] == name).unwrap();

    let alpha = by("Alpha");
    assert_eq!(alpha["latest_season"], 2);
    assert_eq!(alpha["video"], json!([{ "first": "01", "last": "05" }]));
    // Episode 3's subtitle arrived after the first check.
    assert_eq!(alpha["subtitle"], json!([{ "first": "01", "last": "05" }]));
    assert_eq!(alpha["subtitle_coverage"], "all");
    assert_eq!(alpha["subtitle_check_needed"], false);
    assert_eq!(alpha["added_at"], Value::Null);
    assert_eq!(alpha["video_added_at"], Value::Null);
    assert_eq!(alpha["subtitle_added_at"], later);
    assert_eq!(alpha["watch_folder"]["id"], folder.id);
    assert_eq!(alpha["watch_folder"]["path"], text(&root));
    assert_eq!(alpha["missing"], false);

    let beta = by("Beta");
    assert_eq!(beta["subtitle"], json!([]));
    assert_eq!(beta["subtitle_coverage"], "none");
    assert_eq!(beta["subtitle_check_needed"], true);

    let delta = by("Delta");
    assert_eq!(delta["added_at"], later);
    assert_eq!(delta["video_added_at"], later);
    assert_eq!(delta["subtitle_added_at"], Value::Null);

    let gamma = by("Gamma");
    assert_eq!(gamma["missing"], true);
    assert_eq!(gamma["video"], json!([]));
    assert_eq!(gamma["subtitle_coverage"], Value::Null);
}

#[tokio::test]
async fn an_empty_library_lists_no_works() {
    let lib = Lib::new().await;
    assert!(lib.library_list().await.is_empty());
}

/// About 520 works and 10 000 files: one full reading and recording, an unchanged
/// one, and one with a single new file. The times are printed (run with
/// `--nocapture`) and recorded in the ticket, and not asserted: what is checked
/// is that the counts are right.
#[tokio::test]
async fn a_library_of_520_works_and_10_000_files_is_read_and_recorded() {
    const WORKS: usize = 520;
    let lib = Lib::new().await;
    let root = lib.folder("big");
    let mut files = 0;
    let build = std::time::Instant::now();
    for w in 0..WORKS {
        let name = format!("Work {w:03}");
        for e in 1..=12 {
            touch(&root.join(format!("{name}/Season 01/{name} S01E{e:02}.mkv")));
            files += 1;
        }
        for e in 1..=6 {
            touch(&root.join(format!("{name}/Season 01/{name} S01E{e:02}.ko.ass")));
            files += 1;
        }
        touch(&root.join(format!("{name}/Season 01/folder.jpg")));
        touch(&root.join(format!("{name}/Season 01/{name} S01E13.mkv.part")));
        files += 2;
    }
    eprintln!(
        "built {WORKS} works, {files} files in {:?}",
        build.elapsed()
    );

    // As a library that has been there a while: the periodic scan does not
    // skip directories modified moments ago.
    age_dirs(&root);

    let read = std::time::Instant::now();
    let full = discovery::scan_incremental(&root, None);
    let read_time = read.elapsed();
    let dirs = full.stats.dirs_read;
    assert_eq!(full.result.unwrap().works.len(), WORKS);
    assert_eq!(dirs, 1 + 2 * WORKS);
    let again = std::time::Instant::now();
    let unchanged = discovery::scan_incremental(&root, Some(full.cache));
    let unchanged_time = again.elapsed();
    assert_eq!(unchanged.stats.dirs_read, 0);
    assert_eq!(unchanged.stats.dirs_reused, dirs);

    let first = std::time::Instant::now();
    let folder = lib.register(&root).await;
    let first_time = first.elapsed();
    assert_eq!(lib.works(&folder).await.len(), WORKS);

    // The first cycle after the worker starts reads everything; the next finds
    // nothing changed.
    lib.h.advance(1000);
    let cold = std::time::Instant::now();
    lib.tick().await;
    let cold_time = cold.elapsed();
    lib.h.advance(1000);
    let quiet = std::time::Instant::now();
    lib.tick().await;
    let quiet_time = quiet.elapsed();

    touch(&root.join("Work 007/Season 01/Work 007 S01E14.mkv"));
    lib.h.advance(1000);
    let second = std::time::Instant::now();
    lib.tick().await;
    let second_time = second.elapsed();
    let work = lib.work(&folder, "Work 007").await;
    assert_eq!(work.episodes.len(), 13);
    assert_eq!(
        work.files()["Season 01/Work 007 S01E14.mkv"].added_at,
        Some(lib.h.now())
    );

    let list = std::time::Instant::now();
    let works = lib.library_list().await;
    let list_time = list.elapsed();
    assert_eq!(works.len(), WORKS);
    let seven = works.iter().find(|w| w["name"] == "Work 007").unwrap();
    assert_eq!(
        seven["video"],
        json!([{ "first": "01", "last": "12" }, { "first": "14", "last": "14" }])
    );
    assert_eq!(seven["subtitle"], json!([{ "first": "01", "last": "06" }]));
    assert_eq!(seven["subtitle_coverage"], "some");

    eprintln!(
        "library list (GET /api/library/works in pages of 200, {} works): {list_time:?}",
        works.len()
    );
    eprintln!(
        "scan only ({dirs} directories): all listed {read_time:?}, none changed {unchanged_time:?}; \
         first registration (scan, web checks and record): {first_time:?}; worker cycles (scan \
         and record): first after start {cold_time:?}, nothing changed {quiet_time:?}, \
         one new file {second_time:?}"
    );
}
