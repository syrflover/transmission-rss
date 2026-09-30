use std::process::ExitCode;

use tokio::net::TcpListener;
use transmission_rss::web::{self, env::WebEnv};

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

    let listener = TcpListener::bind(env.addr)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", env.addr))?;
    println!(
        "trss-web listening on http://{} (static files: {})",
        env.addr,
        env.static_dir.display()
    );

    axum::serve(listener, web::router(&env.static_dir))
        .with_graceful_shutdown(web::shutdown_signal())
        .await
        .map_err(|e| e.to_string())
}
