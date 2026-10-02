use std::{path::PathBuf, process::ExitCode};

use tokio_util::sync::CancellationToken;
use trss_anissia::{Anissia, AnissiaConfig};
use trss_core::{db::DB_PATH_ENV, lock_path_for, Db};
use trss_legacy::{
    anissia::{self, AnissiaQueue},
    artwork::{self, AnilistConfig, AppData, Artwork},
    seasons::{self, Seasons},
    store::anissia::AnissiaStore,
};
use trss_worker::{Worker, WorkerEnv};

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

    let artwork = Artwork::new(db.clone(), Some(AppData::for_database(&db_path)), anilist);
    let season_info = Seasons::over(db.clone(), &artwork);
    let anissia = AnissiaQueue::new(
        Anissia::with_defaults(db.clone(), anissia_config),
        AnissiaStore::new(db.clone()),
    );
    let worker = Worker::new(db, &env, lock_path_for(&db_path)).map_err(|e| e.to_string())?;

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

    println!(
        "trss-worker started: a cycle every {}s (database: {})",
        env.interval.as_secs(),
        db_path.display()
    );

    worker.run(cancel).await;
    let _ = queue.await;
    let _ = season_queue.await;
    let _ = anissia_queue.await;

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
