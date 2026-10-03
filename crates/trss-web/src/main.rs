use std::process::ExitCode;

use std::path::PathBuf;
use tokio::net::TcpListener;

use trss_anilist::AnilistConfig;
use trss_anissia::{Anissia, AnissiaConfig};
use trss_core::{db::DB_PATH_ENV, wake::wake_path_for, Db};
use trss_library::artwork::{AppData, Artwork};
use trss_web::{self as web, env::WebEnv, AppState};

#[tokio::main]
async fn main() -> ExitCode {
    dotenv::dotenv().ok();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("trss-web: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let env = WebEnv::from_env().map_err(|e| e.to_string())?;

    let index = env.static_dir.join("index.html");
    if !index.is_file() {
        return Err(format!(
            "no frontend build at {} (set {} to the directory produced by `bun run build` in web/)",
            env.static_dir.display(),
            web::env::STATIC_DIR_VAR,
        ));
    }

    let anilist = AnilistConfig::from_env()?;
    let anissia = AnissiaConfig::from_env()?;
    let db_path: PathBuf = std::env::var_os(DB_PATH_ENV)
        .ok_or_else(|| format!("environment variable {DB_PATH_ENV} is not set"))?
        .into();
    let db = Db::open(&db_path).await.map_err(|e| {
        format!("cannot open the app database (set {DB_PATH_ENV} to a file on a local volume): {e}")
    })?;
    // Cover images live in the app data folder: the database's folder.
    let app_data = AppData::for_database(&db_path);
    let receive = trss_jobs::ReceiveArea::in_app_data(app_data.root());
    let artwork = Artwork::new(db.clone(), Some(app_data), anilist);

    let listener = TcpListener::bind(env.addr)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", env.addr))?;
    println!(
        "trss-web listening on http://{} (static files: {})",
        env.addr,
        env.static_dir.display()
    );

    let state = AppState::new(db.clone())
        .with_artwork(artwork)
        .with_anissia(Anissia::with_defaults(db, anissia))
        .with_receive_area(&receive)
        .with_worker_wake(wake_path_for(&db_path));
    // What a killed process, or an upload cut short in this one, left in the
    // receive area (a staging folder, or the folder of a job that was never
    // recorded). This process takes the uploads, so it sweeps: at start and
    // every hour. Anything an upload or a receipt going on owns is younger
    // than the hour, or has a record.
    let sweeping = state.uploads.keep_sweeping(
        std::time::Duration::from_secs(3600),
        std::time::Duration::from_secs(3600),
        |result| match result {
            Ok(removed) => println!("trss-web: removed {removed} abandoned upload folders"),
            Err(e) => eprintln!("trss-web: cannot sweep the receive area: {e}"),
        },
    );
    let served = axum::serve(listener, web::router(&env.static_dir, state))
        .with_graceful_shutdown(web::shutdown_signal())
        .await
        .map_err(|e| e.to_string());
    sweeping.abort();
    served
}
