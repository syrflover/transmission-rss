//! `trss-browserd`: the launcher in the browser container (see
//! [`trss_browser::launcher`]).

use std::process::ExitCode;

use tokio_util::sync::CancellationToken;
use trss_browser::launcher::{self, api, xvfb, Config, Launcher};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("trss-browserd: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let config = Config::from_env().map_err(|e| e.to_string())?;
    let launcher = Launcher::open(config.clone())
        .await
        .map_err(|e| format!("cannot prepare {}: {e}", config.runs_dir.display()))?;

    let cancel = CancellationToken::new();
    // The display outlives the runs on it.
    let display_cancel = CancellationToken::new();
    let display = tokio::spawn(xvfb::supervise(
        config.display.clone(),
        launcher::SCREEN.to_owned(),
        display_cancel.clone(),
    ));
    tokio::spawn(shutdown_on_signal(cancel.clone()));

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", config.bind))?;
    println!(
        "trss-browserd listening on {} (at most {} runs, chromium: {})",
        config.bind,
        config.max_runs,
        config.chromium.display()
    );
    // The runs end as soon as shutdown is asked for: their proxied sockets
    // close, and the server can finish without waiting for them.
    let closer = tokio::spawn({
        let (launcher, cancel) = (launcher.clone(), cancel.clone());
        async move {
            cancel.cancelled().await;
            launcher.reset().await;
        }
    });
    let serve = axum::serve(listener, api::router(launcher.clone())).with_graceful_shutdown({
        let cancel = cancel.clone();
        async move { cancel.cancelled().await }
    });
    tokio::select! {
        served = serve => served.map_err(|e| e.to_string())?,
        _ = async {
            cancel.cancelled().await;
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        } => eprintln!("trss-browserd: connections did not close in time"),
    }

    // Chromium first, then the display it was open on.
    cancel.cancel();
    let _ = closer.await;
    display_cancel.cancel();
    let _ = display.await;
    println!("trss-browserd stopped");
    Ok(())
}

/// Cancels `cancel` on SIGTERM or SIGINT.
async fn shutdown_on_signal(cancel: CancellationToken) {
    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        Ok(signal) => signal,
        Err(err) => {
            eprintln!("trss-browserd: cannot listen for SIGTERM: {err}");
            return;
        }
    };
    tokio::select! {
        _ = sigterm.recv() => println!("SIGTERM received, shutting down"),
        _ = tokio::signal::ctrl_c() => println!("SIGINT received, shutting down"),
    }
    cancel.cancel();
}
