use std::{path::PathBuf, process::ExitCode};

use tokio_util::sync::CancellationToken;
use trss_anilist::AnilistConfig;
use trss_anissia::{Anissia, AnissiaConfig};
use trss_collect::{
    anissia::{self, captions::CaptionObserver, AnissiaQueue},
    store::anissia::AnissiaStore,
};
use trss_core::{db::DB_PATH_ENV, lock_path_for, wake::wake_path_for, Db};
use trss_jobs::{JobStore, ReceiveArea, Runner};
use trss_library::{
    artwork::{self, AppData, Artwork},
    seasons::{self, Seasons},
};
use trss_subtitles::{
    auth::BrowserAuth, blogger::BloggerSource, drive::Drive, erulabo::ErulaboSource,
    fake::FakeSource, naver::NaverSource, tistory::TistorySource, winpng::BrowserReader, Sources,
};
use trss_worker::{env::FAKE_SUBTITLE_SOURCE_VAR, Worker, WorkerEnv};

#[tokio::main]
async fn main() -> ExitCode {
    dotenv::dotenv().ok();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("trss-worker: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let env = WorkerEnv::from_env().map_err(|e| e.to_string())?;
    let anilist = AnilistConfig::from_env()?;
    let anissia_config = AnissiaConfig::from_env()?;

    let db_path: PathBuf = std::env::var_os(DB_PATH_ENV)
        .ok_or_else(|| format!("environment variable {DB_PATH_ENV} is not set"))?
        .into();
    let db = Db::open(&db_path).await.map_err(|e| {
        format!("cannot open the app database (set {DB_PATH_ENV} to a file on a local volume): {e}")
    })?;

    let app_data = AppData::for_database(&db_path);
    // The real sites are always on; the fake one only when asked for. One
    // Drive serves every source that links Drive files, so their requests to
    // Drive's hosts are spaced together. erulabo's files come only through
    // the site's check in the server browser: without one its posts wait.
    let drive = Drive::new();
    let mut sources = Sources::none()
        .with_tistory(TistorySource::new(drive.clone()))
        .with_blogger(BloggerSource::new(drive.clone()))
        .with_naver(NaverSource::new(drive))
        .with_erulabo(ErulaboSource::new());
    if std::env::var(FAKE_SUBTITLE_SOURCE_VAR).as_deref() == Ok("1") {
        println!("The fake subtitle source is on ({FAKE_SUBTITLE_SOURCE_VAR})");
        sources = sources.with_fake(FakeSource);
    }
    // One worker at a time uses the browser container: a second one would
    // reset it and end the first one's runs. The lock lives as long as this
    // process (`_browser_lock`).
    let (browser, _browser_lock) = match &env.browser {
        Some(browser_env) => match trss_worker::browser::take_lock(&db_path)
            .map_err(|e| format!("cannot take the lock of the server browser: {e}"))?
        {
            Some(lock) => (
                Some(
                    trss_worker::browser::connect(
                        db.clone(),
                        browser_env,
                        trss_core::system_clock(),
                    )
                    .await?,
                ),
                Some(lock),
            ),
            None => {
                eprintln!(
                    "trss-worker: another worker uses the browser container; this one runs without a server browser"
                );
                (None, None)
            }
        },
        None => (None, None),
    };
    let jobs = Runner::new(
        JobStore::new(db.clone()),
        sources,
        ReceiveArea::in_app_data(app_data.root()),
        trss_core::system_clock(),
    );
    // The posts whose subtitle is in WinPNG images are read by the server
    // browser, and the posts whose file comes after a person's check on the
    // site are brought to that check in it; a worker without one leaves them
    // waiting.
    let jobs = match &browser {
        Some(pool) => jobs
            .with_winpng(BrowserReader::shared(pool.clone()))
            .with_auth(BrowserAuth::shared(pool.clone())),
        None => jobs,
    };
    let artwork = Artwork::new(db.clone(), Some(app_data), anilist);
    let season_info = Seasons::over(db.clone(), &artwork);
    let anissia_client = Anissia::with_defaults(db.clone(), anissia_config);
    let anissia = AnissiaQueue::new(anissia_client.clone(), AnissiaStore::new(db.clone()));
    let captions = CaptionObserver::new(anissia_client, AnissiaStore::new(db.clone()));
    let mut worker = Worker::new(db, &env, lock_path_for(&db_path))
        .map_err(|e| e.to_string())?
        .with_captions(captions.clone())
        .with_jobs(jobs)
        .with_season_info(season_info.stored())
        .with_wake_socket(wake_path_for(&db_path));
    if let Some(pool) = browser {
        println!("The server browser is on (a pool over the browser container)");
        worker = worker.with_browser(pool);
    }

    let cancel = CancellationToken::new();
    tokio::spawn(shutdown_on_signal(cancel.clone()));

    // Work covers: AniList searches and image fetches, one at a time, beside
    // the collection loop and outside its lock.
    let queue = tokio::spawn({
        let cancel = cancel.clone();
        let lock = artwork::queue::lock_path_for(&db_path);
        async move { artwork.run_queue(lock, cancel).await }
    });

    // Season info: linking a first season by its folder name and the daily
    // refresh of entries that are not finished, on the same pace.
    let season_queue = tokio::spawn({
        let cancel = cancel.clone();
        let lock = seasons::queue::lock_path_for(&db_path);
        async move { season_info.run_queue(lock, cancel).await }
    });

    // Subscribed anime: the daily refresh of their schedule snapshots, on
    // Anissia's own pace.
    let anissia_queue = tokio::spawn({
        let cancel = cancel.clone();
        let lock = anissia::lock_path_for(&db_path);
        async move { anissia.run_queue(lock, cancel).await }
    });

    // The subtitle lines of every Anissia anime: the recent list every 30
    // minutes, whatever the collection cycle is doing, under its own lock.
    let caption_queue = tokio::spawn({
        let cancel = cancel.clone();
        let lock = anissia::captions::lock_path_for(&db_path);
        async move { captions.run_queue(lock, cancel).await }
    });

    println!(
        "trss-worker started: a cycle every {}s (database: {})",
        env.interval.as_secs(),
        db_path.display()
    );

    worker.run(cancel).await;
    let _ = queue.await;
    let _ = season_queue.await;
    let _ = anissia_queue.await;
    let _ = caption_queue.await;

    println!("trss-worker stopped");
    Ok(())
}

/// Cancels `cancel` on SIGTERM or SIGINT, so a cycle in progress can wind down.
async fn shutdown_on_signal(cancel: CancellationToken) {
    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        Ok(signal) => signal,
        Err(err) => {
            eprintln!("trss-worker: cannot listen for SIGTERM: {err}");
            return;
        }
    };

    tokio::select! {
        _ = sigterm.recv() => println!("SIGTERM received, shutting down"),
        _ = tokio::signal::ctrl_c() => println!("SIGINT received, shutting down"),
    }
    cancel.cancel();
}
