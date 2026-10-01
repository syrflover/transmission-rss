use std::process::ExitCode;

use std::path::PathBuf;
use tokio::net::TcpListener;

use transmission_rss::{
    anissia::{Anissia, AnissiaConfig},
    artwork::{AnilistConfig, AppData, Artwork},
    store::{db::DB_PATH_ENV, Db},
    web::{self, env::WebEnv, AppState},
};

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
    let artwork = Artwork::new(db.clone(), Some(AppData::for_database(&db_path)), anilist);

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
        .with_anissia(Anissia::with_defaults(db, anissia));
    axum::serve(listener, web::router(&env.static_dir, state))
        .with_graceful_shutdown(web::shutdown_signal())
        .await
        .map_err(|e| e.to_string())
}
