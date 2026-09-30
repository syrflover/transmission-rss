use std::{path::PathBuf, process::ExitCode};

use tokio_util::sync::CancellationToken;
use transmission_rss::{
    store::{db::DB_PATH_ENV, Db},
    worker::{lock_path_for, Worker, WorkerEnv},
};

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

    let db_path: PathBuf = std::env::var_os(DB_PATH_ENV)
        .ok_or_else(|| format!("environment variable {DB_PATH_ENV} is not set"))?
        .into();
    let db = Db::open(&db_path).await.map_err(|e| {
        format!("cannot open the app database (set {DB_PATH_ENV} to a file on a local volume): {e}")
    })?;

    let worker = Worker::new(db, &env, lock_path_for(&db_path)).map_err(|e| e.to_string())?;

    let cancel = CancellationToken::new();
    tokio::spawn(shutdown_on_signal(cancel.clone()));

    println!(
        "trss-worker started: a cycle every {}s (database: {})",
        env.interval.as_secs(),
        db_path.display()
    );

    worker.run(cancel).await;

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
